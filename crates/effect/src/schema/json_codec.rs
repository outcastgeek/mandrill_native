//! JSON codecs for primitives and composites.

use super::{
    Decimal, IssueSegment, JsonValue, PrimitiveShape, PureCodec, SchemaAst, SchemaIssue,
    SchemaIssueKind, SchemaShape, StringPattern, ValueKind,
};
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
    PureCodec::with_ast(
        SchemaAst::primitive(PrimitiveShape::Null),
        |value| {
            if matches!(value, JsonValue::Null) {
                Ok(())
            } else {
                Err(mismatch(ValueKind::Null, value))
            }
        },
        |_| Ok(JsonValue::Null),
    )
    .with_type_validator(|_| Ok(()))
    .with_writer(|_, writer| writer.null())
}

/// A boolean codec with independent borrowed validation, direct writing and semantic equality.
#[must_use]
pub fn json_bool() -> PureCodec<bool, JsonValue> {
    PureCodec::with_ast(
        SchemaAst::primitive(PrimitiveShape::Boolean),
        |value| {
            if let JsonValue::Bool(boolean) = value {
                Ok(*boolean)
            } else {
                Err(mismatch(ValueKind::Boolean, value))
            }
        },
        |value| Ok(JsonValue::Bool(*value)),
    )
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
    let codec = PureCodec::with_ast(
        SchemaAst::primitive(PrimitiveShape::Number),
        |value| {
            if let JsonValue::Number(number) = value {
                number
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange))
            } else {
                Err(mismatch(ValueKind::Number, value))
            }
        },
        |value| {
            serde_json::Number::from_f64(*value)
                .map(JsonValue::Number)
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::NonFinite))
        },
    );
    let codec = codec.with_type_validator(|value: &f64| {
        if value.is_finite() {
            Ok(())
        } else {
            Err(SchemaIssue::new(SchemaIssueKind::NonFinite))
        }
    });
    if has_writer {
        codec.with_writer(|value: &f64, writer| {
            let number = serde_json::Number::from_f64(*value)
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::NonFinite))?;
            writer.number(&number.to_string())
        })
    } else {
        codec
    }
}

/// Represent Decimal with its existing checked textual encoding, retaining independent validation and JSON writing.
#[must_use]
pub fn json_decimal() -> PureCodec<Decimal, JsonValue> {
    let has_writer = true;
    let codec = PureCodec::with_ast(
        SchemaAst::new(SchemaShape::Checked {
            source: SchemaAst::primitive(PrimitiveShape::String),
            name: Arc::from(DECIMAL_SYNTAX),
        }),
        |encoded| match encoded {
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
        },
        |value| Ok(JsonValue::String(value.as_str().to_owned())),
    );
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
    let codec = PureCodec::with_ast(
        integer.ast().clone(),
        move |encoded| integer.decode_ref(encoded).map(UtcMillis::new),
        |value| Ok(JsonValue::Number(serde_json::Number::from(value.value()))),
    );
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
    PureCodec::with_ast(
        SchemaAst::new(SchemaShape::Checked {
            source: bytes.ast().clone(),
            name: Arc::from(STABLE_KEY_ENCODING),
        }),
        move |encoded| {
            let encoded = bytes.decode_ref(encoded)?;
            StableKey::from_encoded(Arc::from(encoded)).map_err(|_| {
                SchemaIssue::new(SchemaIssueKind::InvalidValue {
                    code: Arc::from(STABLE_KEY_ENCODING),
                })
            })
        },
        |value| {
            Ok(JsonValue::Array(
                value
                    .as_bytes()
                    .iter()
                    .map(|byte| JsonValue::Number(serde_json::Number::from(u64::from(*byte))))
                    .collect(),
            ))
        },
    )
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
#[must_use]
pub fn json_literal(value: Arc<JsonValue>) -> PureCodec<(), JsonValue> {
    let ast = SchemaAst::new(SchemaShape::Literal(Arc::clone(&value)));
    let decode = Arc::clone(&value);
    let write = Arc::clone(&value);
    let has_writer = true;
    let codec = PureCodec::with_ast(
        ast,
        move |input| {
            if input == decode.as_ref() {
                Ok(())
            } else {
                Err(SchemaIssue::new(SchemaIssueKind::LiteralMismatch))
            }
        },
        move |_| Ok(value.as_ref().clone()),
    );
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
