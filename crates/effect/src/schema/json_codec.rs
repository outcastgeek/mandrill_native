//! JSON codecs for primitives and composites.

use super::{
    Decimal, IssueSegment, JsonValue, PrimitiveShape, PureCodec, SchemaAst, SchemaIssue,
    SchemaIssueKind, SchemaShape, StringPattern, ValueKind,
};
use crate::codec;
use crate::duration::UtcMillis;
use crate::primary_key::{KeyDomain, StableKey};
use std::sync::Arc;

// Stable diagnostic names for encoded-side refinements. They identify the
// same rule in AST metadata and typed issues, independent of display text.
const DECIMAL_SYNTAX: &str = "decimal_syntax";
const STABLE_KEY_ENCODING: &str = "stable_key_encoding";

#[must_use]
/// Classify a JSON value for typed mismatch diagnostics.
///
/// # Parameters
/// - `value` (`&JsonValue`): Value read, transformed, encoded, or stored by this operation.
///
/// # Returns
/// Type: `ValueKind`.
pub fn value_kind(value: &JsonValue) -> ValueKind {
    match value {
        JsonValue::Null => ValueKind::Null,
        JsonValue::Bool(_) => ValueKind::Boolean,
        JsonValue::Number(number) if number.is_i64() || number.is_u64() => ValueKind::Integer,
        JsonValue::Number(_) => ValueKind::Number,
        JsonValue::String(_) => ValueKind::String,
        JsonValue::Array(_) => ValueKind::Sequence,
        JsonValue::Object(_) => ValueKind::Record,
    }
}

///
/// # Parameters
/// - `expected` (`ValueKind`): Value or revision required for the proposed state transition.
/// - `value` (`&JsonValue`): Value read, transformed, encoded, or stored by this operation.
///
/// # Returns
/// Type: `SchemaIssue`.
/// Returns `SchemaIssue`.
fn mismatch(expected: ValueKind, value: &JsonValue) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::TypeMismatch {
        expected,
        actual: value_kind(value),
    })
}

/// Decode JSON null as unit and encode unit as null, with independent validation and direct writing.
#[must_use]
pub fn json_null() -> PureCodec<(), JsonValue> {
    codec! { pure; ast: SchemaAst::primitive(PrimitiveShape::Null), decode: |value| {
        if matches!(value, JsonValue::Null) {
            Ok(())
        } else {
            Err(mismatch(ValueKind::Null, value))
        }
    }, encode: |_| Ok(JsonValue::Null), }
    .with_type_validator(|_| Ok(()))
    .with_writer(|_, writer| writer.null())
}

/// A boolean codec with independent borrowed validation, direct writing and semantic equality.
#[must_use]
pub fn json_bool() -> PureCodec<bool, JsonValue> {
    codec! { pure; ast: SchemaAst::primitive(PrimitiveShape::Boolean), decode: |value| {
        if let JsonValue::Bool(boolean) = value {
            Ok(*boolean)
        } else {
            Err(mismatch(ValueKind::Boolean, value))
        }
    }, encode: |value| Ok(JsonValue::Bool(*value)), }
    .with_type_validator(|_| Ok(()))
    .with_writer(|value, writer| writer.boolean(*value))
    .with_equivalence(|left, right| left == right)
}

/// A JSON string codec; configured calls bound Unicode scalar length, with independent validation, writing and equality.
#[must_use]
pub fn json_string() -> PureCodec<String, JsonValue> {
    PureCodec::with_context_ast(
        SchemaAst::primitive(PrimitiveShape::String),
        |value, context| {
            if let JsonValue::String(text) = value {
                check_string_budget(text, context)?;
                Ok(text.clone())
            } else {
                Err(mismatch(ValueKind::String, value))
            }
        },
        |value, context| {
            check_string_budget(value, context)?;
            Ok(JsonValue::String(value.clone()))
        },
    )
    .with_context_type_validator(|value, context| check_string_budget(value, context))
    .with_writer(|value, writer| writer.string(value))
    .with_equivalence(|left, right| left == right)
}

/// A decoded string retaining the complete prepared document that owns its bytes.
pub struct PreparedJsonString {
    owner: super::SharedPreparedJsonValue,
}
impl PreparedJsonString {
    /// Copy a name into an admitted managed string document without JSON text roundtripping.
    pub fn copy_from(text: &str, parser: &super::PreparedJsonParserPool, policy: super::ParsePolicy) -> Result<Self, SchemaIssue> {
        parser.prepare_name(text, policy)
    }
    pub(super) fn new(owner: super::SharedPreparedJsonValue) -> Result<Self, SchemaIssue> {
        if owner.view().as_str().is_none() {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        Ok(Self { owner })
    }
    /// Borrow the retained decoded UTF-8 string.
    #[must_use]
    #[allow(clippy::expect_used)] // The private constructor proves this immutable subtree is a string.
    pub fn as_str(&self) -> &str {
        self.owner
            .view()
            .as_str()
            .expect("prepared string invariant")
    }
    /// Retain another owner of the complete backing document.
    pub fn try_clone(&self) -> Result<Self, crate::memory::MemoryError> {
        Ok(Self {
            owner: self.owner.try_clone()?,
        })
    }
    pub(super) fn retain_value(
        &self,
    ) -> Result<super::SharedPreparedJsonValue, crate::memory::MemoryError> {
        self.owner.try_clone()
    }
}
impl std::ops::Deref for PreparedJsonString {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
pub(super) fn prepared_value_kind(
    value: &super::SharedPreparedJsonValue,
) -> Result<ValueKind, SchemaIssue> {
    prepared_view_kind(value.view())
}
#[doc(hidden)]
pub fn prepared_view_kind(value: super::PreparedJsonValue<'_>) -> Result<ValueKind, SchemaIssue> {
    use super::PreparedJsonKind;
    Ok(
        match value
            .kind()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?
        {
            PreparedJsonKind::Null => ValueKind::Null,
            PreparedJsonKind::Bool(_) => ValueKind::Boolean,
            PreparedJsonKind::Number => {
                let token = value
                    .number_token()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                let number = super::managed_number::ManagedNumber::parse(token)
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                if number.as_i64().is_some() || number.as_u64().is_some() {
                    ValueKind::Integer
                } else {
                    ValueKind::Number
                }
            }
            PreparedJsonKind::String => ValueKind::String,
            PreparedJsonKind::Array => ValueKind::Sequence,
            PreparedJsonKind::Object => ValueKind::Record,
        },
    )
}
/// Decode and encode an owned string view without copying its document backing.
#[must_use]
pub fn json_prepared_string() -> PureCodec<PreparedJsonString, super::SharedPreparedJsonValue> {
    PureCodec::with_context_ast(
        SchemaAst::primitive(PrimitiveShape::String),
        |value: &super::SharedPreparedJsonValue, context| {
            let Some(text) = value.view().as_str() else {
                return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                    expected: ValueKind::String,
                    actual: prepared_value_kind(value)?,
                }));
            };
            check_string_budget(text, context)?;
            let owner = value
                .try_clone()
                .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?;
            PreparedJsonString::new(owner)
        },
        |value: &PreparedJsonString, context| {
            check_string_budget(value.as_str(), context)?;
            value
                .owner
                .try_clone()
                .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))
        },
    )
    .with_context_type_validator(|value: &PreparedJsonString, context| {
        check_string_budget(value.as_str(), context)
    })
    .with_writer(|value, writer| writer.string(value.as_str()))
    .with_equivalence(|left, right| left.as_str() == right.as_str())
}

fn prepared_mismatch(expected: ValueKind, value: &super::SharedPreparedJsonValue) -> SchemaIssue {
    match prepared_value_kind(value) {
        Ok(actual) => SchemaIssue::new(SchemaIssueKind::TypeMismatch { expected, actual }),
        Err(issue) => issue,
    }
}
fn prepared_number(
    value: &super::SharedPreparedJsonValue,
) -> Result<Option<super::managed_number::ManagedNumber<'_>>, SchemaIssue> {
    let Some(token) = value.view().number_token() else {
        return Ok(None);
    };
    super::managed_number::ManagedNumber::parse(token)
        .map(Some)
        .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))
}
fn check_prepared_number_digits(
    number: &super::managed_number::ManagedNumber<'_>,
    context: Option<&super::ParseContext>,
) -> Result<(), SchemaIssue> {
    let Some(context) = context else {
        return Ok(());
    };
    let digits = number.digits();
    let limit = context.policy().integer_digits.get();
    if digits > limit {
        Err(SchemaIssue::new(SchemaIssueKind::ResourceDemand {
            resource: super::SchemaResource::IntegerDigits,
            limit,
            observed: digits,
        }))
    } else {
        Ok(())
    }
}
/// Managed null codec with explicit document and parser policy owners for encoding.
#[must_use]
pub fn json_prepared_null(
    pool: Arc<super::PreparedJsonParserPool>,
    policy: super::ParsePolicy,
) -> PureCodec<(), super::SharedPreparedJsonValue> {
    let writer = json_null();
    let ast = writer.ast().clone();
    PureCodec::with_context_ast(
        ast,
        |value: &super::SharedPreparedJsonValue, _| {
            if value.view().kind() == Some(super::PreparedJsonKind::Null) {
                Ok(())
            } else {
                Err(prepared_mismatch(ValueKind::Null, value))
            }
        },
        move |value: &(), context| {
            pool.encode_with(
                &writer,
                value,
                super::parse_context::policy(context, policy),
            )
        },
    )
    .with_type_validator(|_| Ok(()))
    .with_writer(|_, writer| writer.null())
}
/// Managed boolean codec with explicit document and parser policy owners for encoding.
#[must_use]
pub fn json_prepared_bool(
    pool: Arc<super::PreparedJsonParserPool>,
    policy: super::ParsePolicy,
) -> PureCodec<bool, super::SharedPreparedJsonValue> {
    let writer = json_bool();
    let ast = writer.ast().clone();
    PureCodec::with_context_ast(
        ast,
        |value: &super::SharedPreparedJsonValue, _| match value.view().kind() {
            Some(super::PreparedJsonKind::Bool(value)) => Ok(value),
            _ => Err(prepared_mismatch(ValueKind::Boolean, value)),
        },
        move |value: &bool, context| {
            pool.encode_with(
                &writer,
                value,
                super::parse_context::policy(context, policy),
            )
        },
    )
    .with_type_validator(|_| Ok(()))
    .with_writer(|value, writer| writer.boolean(*value))
    .with_equivalence(|left, right| left == right)
}
/// Managed signed integer codec with explicit document and parser policy owners.
#[must_use]
pub fn json_prepared_i64(
    pool: Arc<super::PreparedJsonParserPool>,
    policy: super::ParsePolicy,
) -> PureCodec<i64, super::SharedPreparedJsonValue> {
    let writer = json_i64();
    let ast = writer.ast().clone();
    PureCodec::with_context_ast(
        ast,
        |value: &super::SharedPreparedJsonValue, context| {
            let number = prepared_number(value)?
                .ok_or_else(|| prepared_mismatch(ValueKind::Integer, value))?;
            check_prepared_number_digits(&number, context)?;
            number
                .as_i64()
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange))
        },
        move |value: &i64, context| {
            let mut buffer = std::fmt::NumBuffer::new();
            check_digit_budget(value.format_into(&mut buffer), context)?;
            pool.encode_with(
                &writer,
                value,
                super::parse_context::policy(context, policy),
            )
        },
    )
    .with_context_type_validator(|value: &i64, context| {
        let mut buffer = std::fmt::NumBuffer::new();
        check_digit_budget(value.format_into(&mut buffer), context)
    })
    .with_writer(|value, writer| writer.i64(*value))
    .with_equivalence(|left, right| left == right)
}
/// Managed unsigned integer codec with explicit document and parser policy owners.
#[must_use]
pub fn json_prepared_u64(
    pool: Arc<super::PreparedJsonParserPool>,
    policy: super::ParsePolicy,
) -> PureCodec<u64, super::SharedPreparedJsonValue> {
    let writer = json_u64();
    let ast = writer.ast().clone();
    PureCodec::with_context_ast(
        ast,
        |value: &super::SharedPreparedJsonValue, context| {
            let number = prepared_number(value)?
                .ok_or_else(|| prepared_mismatch(ValueKind::Integer, value))?;
            check_prepared_number_digits(&number, context)?;
            number
                .as_u64()
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange))
        },
        move |value: &u64, context| {
            let mut buffer = std::fmt::NumBuffer::new();
            check_digit_budget(value.format_into(&mut buffer), context)?;
            pool.encode_with(
                &writer,
                value,
                super::parse_context::policy(context, policy),
            )
        },
    )
    .with_context_type_validator(|value: &u64, context| {
        let mut buffer = std::fmt::NumBuffer::new();
        check_digit_budget(value.format_into(&mut buffer), context)
    })
    .with_writer(|value, writer| writer.u64(*value))
    .with_equivalence(|left, right| left == right)
}
/// Managed finite floating-point codec with explicit document and parser policy owners.
#[must_use]
pub fn json_prepared_f64(
    pool: Arc<super::PreparedJsonParserPool>,
    policy: super::ParsePolicy,
) -> PureCodec<f64, super::SharedPreparedJsonValue> {
    let writer = json_f64();
    let ast = writer.ast().clone();
    PureCodec::with_context_ast(
        ast,
        |value: &super::SharedPreparedJsonValue, _| {
            let number = prepared_number(value)?
                .ok_or_else(|| prepared_mismatch(ValueKind::Number, value))?;
            number
                .as_f64()
                .filter(|number| number.is_finite())
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange))
        },
        move |value: &f64, context| {
            if !value.is_finite() {
                return Err(SchemaIssue::new(SchemaIssueKind::NonFinite));
            }
            pool.encode_with(
                &writer,
                value,
                super::parse_context::policy(context, policy),
            )
        },
    )
    .with_type_validator(|value: &f64| {
        if value.is_finite() {
            Ok(())
        } else {
            Err(SchemaIssue::new(SchemaIssueKind::NonFinite))
        }
    })
    .with_writer(|value, writer| writer.finite_f64(*value))
}

/// Nullable managed values, retaining the same nested witness rules as `json_nullable`.
#[must_use]
pub fn json_prepared_nullable<A: Send + 'static>(
    value: PureCodec<A, super::SharedPreparedJsonValue>,
    pool: Arc<super::PreparedJsonParserPool>,
    policy: super::ParsePolicy,
) -> PureCodec<Option<A>, super::SharedPreparedJsonValue> {
    let ast = SchemaAst::new(SchemaShape::Union(vec![
        SchemaAst::primitive(PrimitiveShape::Null),
        value.ast().clone(),
    ]));
    let decode = value.clone();
    let validate = value.clone();
    let write = value.clone();
    let has_validator = value.has_type_validator();
    let has_writer = value.has_writer();
    let null_writer = json_null();
    let codec = PureCodec::with_context_ast(
        ast,
        move |encoded: &super::SharedPreparedJsonValue, context| {
            if encoded.view().kind() == Some(super::PreparedJsonKind::Null) {
                Ok(None)
            } else {
                decode
                    .decode_in_ref(encoded, super::parse_context::descend(context)?.as_ref())
                    .map(Some)
            }
        },
        move |decoded: &Option<A>, context| match decoded {
            None => pool.encode_with(
                &null_writer,
                &(),
                super::parse_context::policy(context, policy),
            ),
            Some(item) => {
                value.encode_in_ref(item, super::parse_context::descend(context)?.as_ref())
            }
        },
    );
    let codec = if has_validator {
        codec.with_context_type_validator(move |value: &Option<A>, context| match value {
            None => Ok(()),
            Some(value) => {
                validate.validate_in_ref(value, super::parse_context::descend(context)?.as_ref())
            }
        })
    } else {
        codec
    };
    if has_writer {
        codec.with_writer(move |value, writer| match value {
            None => writer.null(),
            Some(value) => write.write_into(value, writer),
        })
    } else {
        codec
    }
}

/// Managed homogeneous arrays with explicit typed-element and output-document backing.
#[must_use]
pub fn json_prepared_array<A: Send + 'static>(
    element: PureCodec<A, super::SharedPreparedJsonValue>,
    arrays: Arc<super::PreparedJsonArrayPool<A>>,
    output: Arc<super::PreparedJsonParserPool>,
    policy: super::ParsePolicy,
    maximum: Option<std::num::NonZeroUsize>,
) -> PureCodec<super::PreparedJsonArray<A>, super::SharedPreparedJsonValue> {
    let ast = SchemaAst::new(SchemaShape::Sequence {
        element: element.ast().clone(),
        maximum,
    });
    let decode = element.clone();
    let validate = element.clone();
    let write = element.clone();
    let has_validator = element.has_type_validator();
    let has_writer = element.has_writer();
    let codec = PureCodec::with_context_ast(
        ast,
        move |encoded: &super::SharedPreparedJsonValue, context| {
            if encoded.view().kind() != Some(super::PreparedJsonKind::Array) {
                return Err(prepared_mismatch(ValueKind::Sequence, encoded));
            }
            let len = encoded.view().len();
            if context.is_some_and(|ctx| len > ctx.policy().parse.limits.collection.get())
                || maximum.is_some_and(|limit| len > limit.get())
            {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            let mut result = arrays
                .reserve()
                .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?;
            if len > result.capacity() {
                return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                    crate::memory::MemoryError::NoSlot,
                )));
            }
            for index in 0..len {
                let child_context =
                    super::parse_context::child(context, IssueSegment::Index(index))?;
                let child = encoded
                    .try_get_index(index)
                    .map_err(|error| {
                        super::prepared_issue_at(context, SchemaIssue::new(SchemaIssueKind::Memory(error)), IssueSegment::Index(index))
                    })?
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                let item = decode
                    .decode_in_ref(&child, child_context.as_ref())
                    .map_err(|issue| super::prepared_issue_at(context, issue, IssueSegment::Index(index)))?;
                if let Err(item) = result.push(item) {
                    drop(item);
                    return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                        crate::memory::MemoryError::NoSlot,
                    )));
                }
            }
            Ok(result)
        },
        move |decoded: &super::PreparedJsonArray<A>, context| {
            let values = decoded.as_slice();
            if context.is_some_and(|ctx| values.len() > ctx.policy().parse.limits.collection.get())
                || maximum.is_some_and(|limit| values.len() > limit.get())
            {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            output.assemble_array(
                values.len(),
                super::parse_context::policy(context, policy),
                |index| {
                    let item = values
                        .get(index)
                        .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                    let child_context =
                        super::parse_context::child(context, IssueSegment::Index(index))?;
                    element
                        .encode_in_ref(item, child_context.as_ref())
                        .map_err(|issue| super::prepared_issue_at(context, issue, IssueSegment::Index(index)))
                },
            )
        },
    );
    let codec = if has_validator {
        codec.with_context_type_validator(move |values: &super::PreparedJsonArray<A>, context| {
            if context.is_some_and(|ctx| {
                values.as_slice().len() > ctx.policy().parse.limits.collection.get()
            }) || maximum.is_some_and(|limit| values.as_slice().len() > limit.get())
            {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            for (index, value) in values.iter().enumerate() {
                validate
                    .validate_in_ref(
                        value,
                        super::parse_context::child(context, IssueSegment::Index(index))?.as_ref(),
                    )
                    .map_err(|issue| super::prepared_issue_at(context, issue, IssueSegment::Index(index)))?;
            }
            Ok(())
        })
    } else {
        codec
    };
    if has_writer {
        codec.with_writer(move |values, writer| {
            if maximum.is_some_and(|limit| values.as_slice().len() > limit.get()) {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            writer.begin_array()?;
            for (index, value) in values.iter().enumerate() {
                write
                    .write_into(value, writer)
                    .map_err(|issue| writer.issue_at(issue, IssueSegment::Index(index)))?;
            }
            writer.end_array()
        })
    } else {
        codec
    }
}

/// A pair over managed JSON, retaining the ordinary tuple witness rules.
#[must_use]
pub fn json_prepared_tuple2<A: Send + 'static, B: Send + 'static>(
    first: PureCodec<A, super::SharedPreparedJsonValue>,
    second: PureCodec<B, super::SharedPreparedJsonValue>,
    output: Arc<super::PreparedJsonParserPool>,
    policy: super::ParsePolicy,
) -> PureCodec<(A, B), super::SharedPreparedJsonValue> {
    let ast = SchemaAst::new(SchemaShape::Tuple(vec![
        first.ast().clone(),
        second.ast().clone(),
    ]));
    let decode_first = first.clone();
    let decode_second = second.clone();
    let validation_first = first.clone();
    let validation_second = second.clone();
    let write_first = first.clone();
    let write_second = second.clone();
    let has_validator = first.has_type_validator() && second.has_type_validator();
    let has_writer = first.has_writer() && second.has_writer();
    let codec = PureCodec::with_context_ast(
        ast,
        move |encoded: &super::SharedPreparedJsonValue, context| {
            if encoded.view().kind() != Some(super::PreparedJsonKind::Array) {
                return Err(prepared_mismatch(ValueKind::Sequence, encoded));
            }
            let len = encoded.view().len();
            if len != 2 {
                return Err(SchemaIssue::new(SchemaIssueKind::LengthMismatch {
                    expected: 2,
                    actual: len,
                }));
            }
            let left_context = super::parse_context::child(context, IssueSegment::Index(0))?;
            let left = encoded
                .try_get_index(0)
                .map_err(|error| {
                    super::prepared_issue_at(context, SchemaIssue::new(SchemaIssueKind::Memory(error)), IssueSegment::Index(0))
                })?
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
            let left = decode_first
                .decode_in_ref(&left, left_context.as_ref())
                .map_err(|issue| super::prepared_issue_at(context, issue, IssueSegment::Index(0)))?;
            let right_context = super::parse_context::child(context, IssueSegment::Index(1))?;
            let right = encoded
                .try_get_index(1)
                .map_err(|error| {
                    super::prepared_issue_at(context, SchemaIssue::new(SchemaIssueKind::Memory(error)), IssueSegment::Index(1))
                })?
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
            let right = decode_second
                .decode_in_ref(&right, right_context.as_ref())
                .map_err(|issue| super::prepared_issue_at(context, issue, IssueSegment::Index(1)))?;
            Ok((left, right))
        },
        move |(left, right): &(A, B), context| {
            output.assemble_array(2, super::parse_context::policy(context, policy), |index| {
                match index {
                    0 => first
                        .encode_in_ref(
                            left,
                            super::parse_context::child(context, IssueSegment::Index(0))?.as_ref(),
                        )
                        .map_err(|issue| super::prepared_issue_at(context, issue, IssueSegment::Index(0))),
                    1 => second
                        .encode_in_ref(
                            right,
                            super::parse_context::child(context, IssueSegment::Index(1))?.as_ref(),
                        )
                        .map_err(|issue| super::prepared_issue_at(context, issue, IssueSegment::Index(1))),
                    _ => Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)),
                }
            })
        },
    );
    let codec = if has_validator {
        codec.with_context_type_validator(move |(first, second): &(A, B), context| {
            validation_first
                .validate_in_ref(
                    first,
                    super::parse_context::child(context, IssueSegment::Index(0))?.as_ref(),
                )
                .map_err(|issue| super::prepared_issue_at(context, issue, IssueSegment::Index(0)))?;
            validation_second
                .validate_in_ref(
                    second,
                    super::parse_context::child(context, IssueSegment::Index(1))?.as_ref(),
                )
                .map_err(|issue| super::prepared_issue_at(context, issue, IssueSegment::Index(1)))
        })
    } else {
        codec
    };
    if has_writer {
        codec.with_writer(move |(first, second), writer| {
            writer.begin_array()?;
            write_first
                .write_into(first, writer)
                .map_err(|issue| writer.issue_at(issue, IssueSegment::Index(0)))?;
            write_second
                .write_into(second, writer)
                .map_err(|issue| writer.issue_at(issue, IssueSegment::Index(1)))?;
            writer.end_array()
        })
    } else {
        codec
    }
}

/// Constrain JSON strings with the existing prepared StringPattern expression.
///
/// Configured traversal is forwarded through the source string codec. This legacy
/// pattern surface remains distinct from the explicitly budgeted CheckedPattern
/// used by json_string_constraints.
pub fn json_string_pattern(
    pattern: StringPattern,
    issue_code: impl Into<Arc<str>>,
) -> Result<PureCodec<String, JsonValue>, SchemaIssue> {
    let issue_code = issue_code.into();
    if issue_code.is_empty() {
        return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
    }
    let source = json_string();
    let ast = source.ast().derive(SchemaShape::StringPattern {
        source: source.ast().clone(),
        pattern: pattern.clone(),
    });
    let decode = source.clone();
    let decode_pattern = pattern.clone();
    let decode_code = Arc::clone(&issue_code);
    let validate_pattern = pattern.clone();
    let validate_code = Arc::clone(&issue_code);
    let writer_pattern = pattern.clone();
    let writer_code = Arc::clone(&issue_code);
    Ok(PureCodec::with_context_ast(
        ast,
        move |encoded, context| {
            let value =
                decode.decode_in_ref(encoded, super::parse_context::descend(context)?.as_ref())?;
            if decode_pattern.matches(&value) {
                Ok(value)
            } else {
                Err(SchemaIssue::new(SchemaIssueKind::InvalidValue {
                    code: Arc::clone(&decode_code),
                }))
            }
        },
        move |value, context| {
            if pattern.matches(value) {
                source.encode_in_ref(value, super::parse_context::descend(context)?.as_ref())
            } else {
                Err(SchemaIssue::new(SchemaIssueKind::InvalidValue {
                    code: Arc::clone(&issue_code),
                }))
            }
        },
    )
    .with_context_type_validator(move |value, context| {
        check_string_budget(value, context)?;
        if validate_pattern.matches(value) {
            Ok(())
        } else {
            Err(SchemaIssue::new(SchemaIssueKind::InvalidValue {
                code: Arc::clone(&validate_code),
            }))
        }
    })
    .with_writer(move |value, writer| {
        if !writer_pattern.matches(value) {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidValue {
                code: Arc::clone(&writer_code),
            }));
        }
        writer.string(value)
    }))
}

/// Decode/encode the complete signed i64 range losslessly; configured borrowed validation shares the numeric digit bound.
#[must_use]
pub fn json_i64() -> PureCodec<i64, JsonValue> {
    PureCodec::with_context_ast(
        SchemaAst::new(SchemaShape::IntegerRange {
            source: SchemaAst::primitive(PrimitiveShape::Integer),
            minimum: i64::MIN,
            maximum: i64::MAX,
        }),
        |value, context| {
            if let JsonValue::Number(number) = value {
                check_digit_budget(&number.to_string(), context)?;
                number
                    .as_i64()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange))
            } else {
                Err(mismatch(ValueKind::Integer, value))
            }
        },
        |value, context| {
            let mut buffer = std::fmt::NumBuffer::new();
            check_digit_budget(value.format_into(&mut buffer), context)?;
            Ok(JsonValue::Number(serde_json::Number::from(*value)))
        },
    )
    .with_context_type_validator(|value: &i64, context| {
        let mut buffer = std::fmt::NumBuffer::new();
        check_digit_budget(value.format_into(&mut buffer), context)
    })
    .with_writer(|value, writer| writer.i64(*value))
    .with_equivalence(|left, right| left == right)
}

/// Decode/encode the complete unsigned u64 range losslessly, with independent bounded validation and direct writing.
#[must_use]
pub fn json_u64() -> PureCodec<u64, JsonValue> {
    PureCodec::with_context_ast(
        SchemaAst::new(SchemaShape::UnsignedRange {
            source: SchemaAst::primitive(PrimitiveShape::Integer),
            minimum: u64::MIN,
            maximum: u64::MAX,
        }),
        |value, context| {
            if let JsonValue::Number(number) = value {
                check_digit_budget(&number.to_string(), context)?;
                number
                    .as_u64()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange))
            } else {
                Err(mismatch(ValueKind::Integer, value))
            }
        },
        |value, context| {
            let mut buffer = std::fmt::NumBuffer::new();
            check_digit_budget(value.format_into(&mut buffer), context)?;
            Ok(JsonValue::Number(serde_json::Number::from(*value)))
        },
    )
    .with_context_type_validator(|value: &u64, context| {
        let mut buffer = std::fmt::NumBuffer::new();
        check_digit_budget(value.format_into(&mut buffer), context)
    })
    .with_writer(|value, writer| writer.u64(*value))
    .with_equivalence(|left, right| left == right)
}

/// A finite floating-point codec with independent validation and direct writing; non-finite values are rejected.
#[must_use]
pub fn json_f64() -> PureCodec<f64, JsonValue> {
    let has_writer = true;
    let codec = codec! { pure; ast: SchemaAst::primitive(PrimitiveShape::Number), decode: |value| {
        if let JsonValue::Number(number) = value {
            number
                .as_f64()
                .filter(|number| number.is_finite())
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange))
        } else {
            Err(mismatch(ValueKind::Number, value))
        }
    }, encode: |value| {
        serde_json::Number::from_f64(*value)
            .map(JsonValue::Number)
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::NonFinite))
    }, };
    let codec = codec.with_type_validator(|value: &f64| {
        if value.is_finite() {
            Ok(())
        } else {
            Err(SchemaIssue::new(SchemaIssueKind::NonFinite))
        }
    });
    if has_writer {
        codec.with_writer(|value: &f64, writer| writer.finite_f64(*value))
    } else {
        codec
    }
}

/// Represent Decimal with its existing checked textual encoding, retaining independent validation and JSON writing.
#[must_use]
pub fn json_decimal() -> PureCodec<Decimal, JsonValue> {
    let has_writer = true;
    let codec = codec! { pure; ast: SchemaAst::new(SchemaShape::Checked {
        source: SchemaAst::primitive(PrimitiveShape::String),
        name: Arc::from(DECIMAL_SYNTAX),
    }), decode: |encoded| match encoded {
        JsonValue::String(text) => Decimal::parse(text).map_err(|_| {
            SchemaIssue::new(SchemaIssueKind::InvalidValue {
                code: Arc::from(DECIMAL_SYNTAX),
            })
        }),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => Err(mismatch(ValueKind::String, encoded)),
    }, encode: |value| Ok(JsonValue::String(value.as_str().to_owned())), };
    let codec = codec.with_type_validator(|_: &Decimal| Ok(()));
    if has_writer {
        codec.with_writer(|value: &Decimal, writer| writer.string(value.as_str()))
    } else {
        codec
    }
}

/// Represent UtcMillis using its existing checked millisecond JSON encoding and direct-writer witness.
#[must_use]
pub fn json_utc_millis() -> PureCodec<UtcMillis, JsonValue> {
    let integer = json_i64();
    let has_writer = true;
    let codec = codec! { pure; ast: integer.ast().clone(), decode: move |encoded| integer.decode_ref(encoded).map(UtcMillis::new), encode: |value| Ok(JsonValue::Number(serde_json::Number::from(value.value()))), };
    let codec = codec.with_type_validator(|_: &UtcMillis| Ok(()));
    if has_writer {
        codec.with_writer(|value: &UtcMillis, writer| writer.i64(value.value()))
    } else {
        codec
    }
}

/// Represent typed stable-key bytes with their checked JSON codec.
///
/// Decoding validates the key's domain framing; direct writing preserves the same
/// representation rather than choosing a runtime or new wire identity.
#[must_use]
pub fn json_stable_key<D: KeyDomain>() -> PureCodec<StableKey<D>, JsonValue> {
    let bytes = json_bytes(None);
    codec! { pure; ast: SchemaAst::new(SchemaShape::Checked {
        source: bytes.ast().clone(),
        name: Arc::from(STABLE_KEY_ENCODING),
    }), decode: move |encoded| {
        let encoded = bytes.decode_ref(encoded)?;
        StableKey::from_encoded(Arc::from(encoded)).map_err(|_| {
            SchemaIssue::new(SchemaIssueKind::InvalidValue {
                code: Arc::from(STABLE_KEY_ENCODING),
            })
        })
    }, encode: |value| {
        Ok(JsonValue::Array(
            value
                .as_bytes()
                .iter()
                .map(|byte| JsonValue::Number(serde_json::Number::from(u64::from(*byte))))
                .collect(),
        ))
    }, }
    .with_type_validator(|_: &StableKey<D>| Ok(()))
    .with_writer(|value: &StableKey<D>, writer| {
        writer.begin_array()?;
        for byte in value.as_bytes() {
            writer.u64(u64::from(*byte))?;
        }
        writer.end_array()
    })
}

/// Represent None as JSON null and Some through the supplied element codec.
///
/// Null does not invoke an element callback. Non-null directions forward configured
/// traversal; independent validation/writing propagate only from child witnesses.
#[must_use]
pub fn json_nullable<A: Send + 'static>(
    value: PureCodec<A, JsonValue>,
) -> PureCodec<Option<A>, JsonValue> {
    let ast = SchemaAst::new(SchemaShape::Union(vec![
        SchemaAst::primitive(PrimitiveShape::Null),
        value.ast().clone(),
    ]));
    let decode = value.clone();
    let write = value.clone();
    let has_writer = value.has_writer();
    let validate = value.clone();
    let has_validator = value.has_type_validator();
    let codec = PureCodec::with_context_ast(
        ast,
        move |encoded, context| {
            if matches!(encoded, JsonValue::Null) {
                Ok(None)
            } else {
                decode
                    .decode_in_ref(encoded, super::parse_context::descend(context)?.as_ref())
                    .map(Some)
            }
        },
        move |decoded, context| match decoded {
            None => Ok(JsonValue::Null),
            Some(item) => {
                value.encode_in_ref(item, super::parse_context::descend(context)?.as_ref())
            }
        },
    );
    let codec = if has_validator {
        codec.with_context_type_validator(move |value: &Option<A>, context| match value {
            None => Ok(()),
            Some(value) => {
                validate.validate_in_ref(value, super::parse_context::descend(context)?.as_ref())
            }
        })
    } else {
        codec
    };
    if has_writer {
        codec.with_writer(move |value, writer| match value {
            None => writer.null(),
            Some(value) => write.write_into(value, writer),
        })
    } else {
        codec
    }
}

/// Compose a borrowed homogeneous array with an optional explicit width bound.
///
/// Configured traversal also enforces active collection limits and index paths.
/// Independent validation and direct writing are exposed only when the element
/// codec supplies those witnesses; no type-side encoder replay is inferred.
#[must_use]
pub fn json_array<A: Send + 'static>(
    element: PureCodec<A, JsonValue>,
    maximum: Option<std::num::NonZeroUsize>,
) -> PureCodec<Vec<A>, JsonValue> {
    let ast = SchemaAst::new(SchemaShape::Sequence {
        element: element.ast().clone(),
        maximum,
    });
    let decode = element.clone();
    let write = element.clone();
    let has_writer = element.has_writer();
    let validate = element.clone();
    let has_validator = element.has_type_validator();
    let codec = PureCodec::with_context_ast(
        ast,
        move |encoded, context| {
            let JsonValue::Array(items) = encoded else {
                return Err(mismatch(ValueKind::Sequence, encoded));
            };
            if context.is_some_and(|ctx| items.len() > ctx.policy().parse.limits.collection.get())
                || maximum.is_some_and(|limit| items.len() > limit.get())
            {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    decode
                        .decode_in_ref(
                            item,
                            super::parse_context::child(context, IssueSegment::Index(index))?
                                .as_ref(),
                        )
                        .map_err(|issue| issue.at(IssueSegment::Index(index)))
                })
                .collect()
        },
        move |decoded: &Vec<A>, context| {
            if context.is_some_and(|ctx| decoded.len() > ctx.policy().parse.limits.collection.get())
                || maximum.is_some_and(|limit| decoded.len() > limit.get())
            {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            decoded
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    element
                        .encode_in_ref(
                            item,
                            super::parse_context::child(context, IssueSegment::Index(index))?
                                .as_ref(),
                        )
                        .map_err(|issue| issue.at(IssueSegment::Index(index)))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(JsonValue::Array)
        },
    );
    let codec = if has_validator {
        codec.with_context_type_validator(move |values: &Vec<A>, context| {
            if context.is_some_and(|ctx| values.len() > ctx.policy().parse.limits.collection.get())
                || maximum.is_some_and(|limit| values.len() > limit.get())
            {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            for (index, value) in values.iter().enumerate() {
                validate
                    .validate_in_ref(
                        value,
                        super::parse_context::child(context, IssueSegment::Index(index))?.as_ref(),
                    )
                    .map_err(|issue| issue.at(IssueSegment::Index(index)))?;
            }
            Ok(())
        })
    } else {
        codec
    };
    if has_writer {
        codec.with_writer(move |values: &Vec<A>, writer| {
            if maximum.is_some_and(|maximum| values.len() > maximum.get()) {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            writer.begin_array()?;
            for (index, value) in values.iter().enumerate() {
                write
                    .write_into(value, writer)
                    .map_err(|issue| issue.at(IssueSegment::Index(index)))?;
            }
            writer.end_array()
        })
    } else {
        codec
    }
}

/// Represent a pair as exactly two JSON array elements with indexed child issues.
///
/// Configured context is forwarded to both positions. Validation and writing are
/// available only when both children supply genuine corresponding witnesses.
#[must_use]
pub fn json_tuple2<A: Send + 'static, B: Send + 'static>(
    first: PureCodec<A, JsonValue>,
    second: PureCodec<B, JsonValue>,
) -> PureCodec<(A, B), JsonValue> {
    let ast = SchemaAst::new(SchemaShape::Tuple(vec![
        first.ast().clone(),
        second.ast().clone(),
    ]));
    let write_first = first.clone();
    let write_second = second.clone();
    let has_writer = first.has_writer() && second.has_writer();
    let has_validator = first.has_type_validator() && second.has_type_validator();
    let validation_first = first.clone();
    let validation_second = second.clone();
    let decode_first = first.clone();
    let decode_second = second.clone();
    let codec = PureCodec::with_context_ast(
        ast,
        move |encoded, context| {
            let JsonValue::Array(items) = encoded else {
                return Err(mismatch(ValueKind::Sequence, encoded));
            };
            let [left, right] = items.as_slice() else {
                return Err(SchemaIssue::new(SchemaIssueKind::LengthMismatch {
                    expected: 2,
                    actual: items.len(),
                }));
            };
            let left = decode_first
                .decode_in_ref(
                    left,
                    super::parse_context::child(context, IssueSegment::Index(0))?.as_ref(),
                )
                .map_err(|issue| issue.at(IssueSegment::Index(0)))?;
            let right = decode_second
                .decode_in_ref(
                    right,
                    super::parse_context::child(context, IssueSegment::Index(1))?.as_ref(),
                )
                .map_err(|issue| issue.at(IssueSegment::Index(1)))?;
            Ok((left, right))
        },
        move |(left, right), context| {
            let left = first
                .encode_in_ref(
                    left,
                    super::parse_context::child(context, IssueSegment::Index(0))?.as_ref(),
                )
                .map_err(|issue| issue.at(IssueSegment::Index(0)))?;
            let right = second
                .encode_in_ref(
                    right,
                    super::parse_context::child(context, IssueSegment::Index(1))?.as_ref(),
                )
                .map_err(|issue| issue.at(IssueSegment::Index(1)))?;
            Ok(JsonValue::Array(vec![left, right]))
        },
    );
    let codec = if has_validator {
        codec.with_context_type_validator(move |(first, second), context| {
            validation_first
                .validate_in_ref(
                    first,
                    super::parse_context::child(context, IssueSegment::Index(0))?.as_ref(),
                )
                .map_err(|issue| issue.at(IssueSegment::Index(0)))?;
            validation_second
                .validate_in_ref(
                    second,
                    super::parse_context::child(context, IssueSegment::Index(1))?.as_ref(),
                )
                .map_err(|issue| issue.at(IssueSegment::Index(1)))
        })
    } else {
        codec
    };
    if has_writer {
        codec.with_writer(move |(first, second), writer| {
            writer.begin_array()?;
            write_first
                .write_into(first, writer)
                .map_err(|issue| issue.at(IssueSegment::Index(0)))?;
            write_second
                .write_into(second, writer)
                .map_err(|issue| issue.at(IssueSegment::Index(1)))?;
            writer.end_array()
        })
    } else {
        codec
    }
}

/// Represent owned bytes as JSON integer array elements in the inclusive u8 range.
///
/// An explicit maximum and configured collection limits bound width. Independent
/// validation and direct writing preserve the representation without cloning the
/// payload or invoking an ordinary element encoder.
#[must_use]
pub fn json_prepared_bytes(
    arrays: Arc<super::PreparedJsonArrayPool<u8>>,
    output: Arc<super::PreparedJsonParserPool>,
    policy: super::ParsePolicy,
    maximum: Option<std::num::NonZeroUsize>,
) -> PureCodec<super::PreparedJsonArray<u8>, super::SharedPreparedJsonValue> {
    let ast = SchemaAst::new(SchemaShape::Sequence {
        element: SchemaAst::new(SchemaShape::IntegerRange {
            source: SchemaAst::primitive(PrimitiveShape::Integer),
            minimum: i64::from(u8::MIN),
            maximum: i64::from(u8::MAX),
        }),
        maximum,
    });
    PureCodec::with_context_ast(
        ast,
        move |encoded: &super::SharedPreparedJsonValue, context| {
            if encoded.view().kind() != Some(super::PreparedJsonKind::Array) {
                return Err(prepared_mismatch(ValueKind::Sequence, encoded));
            }
            let len = encoded.view().len();
            check_collection_budget(len, context)?;
            if maximum.is_some_and(|limit| len > limit.get()) {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            let mut bytes = arrays
                .reserve()
                .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?;
            if len > bytes.capacity() {
                return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                    crate::memory::MemoryError::NoSlot,
                )));
            }
            let mut child = encoded.view().first_child();
            let mut index = 0usize;
            while let Some(value) = child {
                let item = if let Some(token) = value.number_token() {
                    super::managed_number::ManagedNumber::parse(token)
                        .and_then(|n| n.as_u64())
                        .and_then(|n| u8::try_from(n).ok())
                        .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange))
                } else {
                    Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                        expected: ValueKind::Integer,
                        actual: prepared_view_kind(value)?,
                    }))
                }
                .map_err(|issue| super::prepared_issue_at(context, issue, IssueSegment::Index(index)))?;
                bytes.push(item).map_err(|_| {
                    SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::NoSlot))
                })?;
                child = value.next_sibling();
                index += 1;
            }
            Ok(bytes)
        },
        move |bytes: &super::PreparedJsonArray<u8>, context| {
            check_collection_budget(bytes.as_slice().len(), context)?;
            if maximum.is_some_and(|limit| bytes.as_slice().len() > limit.get()) {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            output.assemble_bytes(
                bytes.as_slice(),
                super::parse_context::policy(context, policy),
            )
        },
    )
    .with_context_type_validator(move |bytes: &super::PreparedJsonArray<u8>, context| {
        check_collection_budget(bytes.as_slice().len(), context)?;
        if maximum.is_some_and(|limit| bytes.as_slice().len() > limit.get()) {
            Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                resource: super::SchemaResource::Width,
            }))
        } else {
            Ok(())
        }
    })
    .with_writer(move |bytes: &super::PreparedJsonArray<u8>, writer| {
        if maximum.is_some_and(|limit| bytes.as_slice().len() > limit.get()) {
            return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                resource: super::SchemaResource::Width,
            }));
        }
        writer.begin_array()?;
        for byte in bytes.as_slice() {
            writer.u64(u64::from(*byte))?;
        }
        writer.end_array()
    })
}

/// Represent ordinary owned bytes as a JSON array of integers in the u8 range.
#[must_use]
pub fn json_bytes(maximum: Option<std::num::NonZeroUsize>) -> PureCodec<Vec<u8>, JsonValue> {
    let ast = SchemaAst::new(SchemaShape::Sequence {
        element: SchemaAst::new(SchemaShape::IntegerRange {
            source: SchemaAst::primitive(PrimitiveShape::Integer),
            minimum: i64::from(u8::MIN),
            maximum: i64::from(u8::MAX),
        }),
        maximum,
    });
    let has_writer = true;
    let codec = PureCodec::with_context_ast(
        ast,
        move |encoded, context| {
            let JsonValue::Array(items) = encoded else {
                return Err(mismatch(ValueKind::Sequence, encoded));
            };
            check_collection_budget(items.len(), context)?;
            if maximum.is_some_and(|limit| items.len() > limit.get()) {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    if let JsonValue::Number(number) = item {
                        number
                            .as_u64()
                            .and_then(|number| u8::try_from(number).ok())
                            .ok_or_else(|| {
                                SchemaIssue::new(SchemaIssueKind::OutOfRange)
                                    .at(IssueSegment::Index(index))
                            })
                    } else {
                        Err(mismatch(ValueKind::Integer, item).at(IssueSegment::Index(index)))
                    }
                })
                .collect()
        },
        move |decoded: &Vec<u8>, context| {
            check_collection_budget(decoded.len(), context)?;
            if maximum.is_some_and(|limit| decoded.len() > limit.get()) {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            Ok(JsonValue::Array(
                decoded
                    .iter()
                    .map(|byte| JsonValue::Number(serde_json::Number::from(u64::from(*byte))))
                    .collect(),
            ))
        },
    );
    let codec = codec.with_context_type_validator(move |values: &Vec<u8>, context| {
        check_collection_budget(values.len(), context)?;
        if maximum.is_some_and(|maximum| values.len() > maximum.get()) {
            Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                resource: super::SchemaResource::Width,
            }))
        } else {
            Ok(())
        }
    });
    if has_writer {
        codec.with_writer(move |values: &Vec<u8>, writer| {
            if maximum.is_some_and(|maximum| values.len() > maximum.get()) {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            writer.begin_array()?;
            for value in values {
                writer.u64(u64::from(*value))?;
            }
            writer.end_array()
        })
    } else {
        codec
    }
}

/// Accept one application-supplied JSON literal and decode it as unit.
///
/// Encoding and the direct writer reproduce the retained literal; the typed unit
/// validator is independent of decoding. Equality follows the existing literal
/// codec's representation comparison.
pub fn json_prepared_literal(
    value: Arc<JsonValue>,
    output: &super::PreparedJsonParserPool,
    policy: super::ParsePolicy,
) -> Result<PureCodec<(), super::SharedPreparedJsonValue>, SchemaIssue> {
    let template = output.copy_native_template(&value, policy)?;
    let writer = template
        .try_clone()
        .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?;
    let ast = SchemaAst::new(SchemaShape::Literal(Arc::clone(&value)));
    let codec = PureCodec::with_context_ast(
        ast,
        move |input: &super::SharedPreparedJsonValue, _| {
            if super::managed_literal::equals(input.view(), &value)? {
                Ok(())
            } else {
                Err(SchemaIssue::new(SchemaIssueKind::LiteralMismatch))
            }
        },
        move |_: &(), _| {
            template
                .try_clone()
                .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))
        },
    );
    Ok(codec
        .with_type_validator(|_| Ok(()))
        .with_writer(move |_, output| writer.view().write_to(output)))
}

/// Accept one application-supplied ordinary JSON literal and decode it as unit.
#[must_use]
pub fn json_literal(value: Arc<JsonValue>) -> PureCodec<(), JsonValue> {
    let ast = SchemaAst::new(SchemaShape::Literal(Arc::clone(&value)));
    let decode = Arc::clone(&value);
    let write = Arc::clone(&value);
    let has_writer = true;
    let codec = codec! { pure; ast: ast, decode: move |input| {
        if input == decode.as_ref() {
            Ok(())
        } else {
            Err(SchemaIssue::new(SchemaIssueKind::LiteralMismatch))
        }
    }, encode: move |_| Ok(value.as_ref().clone()), };
    let codec = codec.with_type_validator(|_| Ok(()));
    if has_writer {
        codec.with_writer(move |_, writer| writer.tree(&write))
    } else {
        codec
    }
}

fn check_string_budget(
    value: &str,
    context: Option<&super::ParseContext>,
) -> Result<(), SchemaIssue> {
    if let Some(context) = context {
        let limit = context.policy().string_length.get();
        let observed = value.chars().count();
        if observed > limit {
            return Err(SchemaIssue::new(SchemaIssueKind::ResourceDemand {
                resource: super::SchemaResource::StringLength,
                limit,
                observed,
            }));
        }
    }
    Ok(())
}

fn check_collection_budget(
    observed: usize,
    context: Option<&super::ParseContext>,
) -> Result<(), SchemaIssue> {
    if let Some(context) = context {
        let limit = context.policy().parse.limits.collection.get();
        if observed > limit {
            return Err(SchemaIssue::new(SchemaIssueKind::ResourceDemand {
                resource: super::SchemaResource::Width,
                limit,
                observed,
            }));
        }
    }
    Ok(())
}

fn check_digit_budget(
    value: &str,
    context: Option<&super::ParseContext>,
) -> Result<(), SchemaIssue> {
    if let Some(context) = context {
        let limit = context.policy().integer_digits.get();
        let observed = value.bytes().filter(u8::is_ascii_digit).count();
        if observed > limit {
            return Err(SchemaIssue::new(SchemaIssueKind::ResourceDemand {
                resource: super::SchemaResource::IntegerDigits,
                limit,
                observed,
            }));
        }
    }
    Ok(())
}
