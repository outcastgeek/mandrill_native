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

fn mismatch(expected: ValueKind, value: &JsonValue) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::TypeMismatch {
        expected,
        actual: value_kind(value),
    })
}

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
}

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
}

#[must_use]
pub fn json_string() -> PureCodec<String, JsonValue> {
    PureCodec::with_ast(
        SchemaAst::primitive(PrimitiveShape::String),
        |value| {
            if let JsonValue::String(text) = value {
                Ok(text.clone())
            } else {
                Err(mismatch(ValueKind::String, value))
            }
        },
        |value| Ok(JsonValue::String(value.clone())),
    )
}

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
    Ok(PureCodec::with_ast(
        ast,
        move |encoded| {
            let value = decode.decode_ref(encoded)?;
            if decode_pattern.matches(&value) {
                Ok(value)
            } else {
                Err(SchemaIssue::new(SchemaIssueKind::InvalidValue {
                    code: Arc::clone(&decode_code),
                }))
            }
        },
        move |value| {
            if pattern.matches(value) {
                source.encode_ref(value)
            } else {
                Err(SchemaIssue::new(SchemaIssueKind::InvalidValue {
                    code: Arc::clone(&issue_code),
                }))
            }
        },
    ))
}

#[must_use]
pub fn json_i64() -> PureCodec<i64, JsonValue> {
    PureCodec::with_ast(
        SchemaAst::new(SchemaShape::IntegerRange {
            source: SchemaAst::primitive(PrimitiveShape::Integer),
            minimum: i64::MIN,
            maximum: i64::MAX,
        }),
        |value| {
            if let JsonValue::Number(number) = value {
                number
                    .as_i64()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange))
            } else {
                Err(mismatch(ValueKind::Integer, value))
            }
        },
        |value| Ok(JsonValue::Number(serde_json::Number::from(*value))),
    )
}

#[must_use]
pub fn json_u64() -> PureCodec<u64, JsonValue> {
    PureCodec::with_ast(
        SchemaAst::new(SchemaShape::UnsignedRange {
            source: SchemaAst::primitive(PrimitiveShape::Integer),
            minimum: u64::MIN,
            maximum: u64::MAX,
        }),
        |value| {
            if let JsonValue::Number(number) = value {
                number
                    .as_u64()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::OutOfRange))
            } else {
                Err(mismatch(ValueKind::Integer, value))
            }
        },
        |value| Ok(JsonValue::Number(serde_json::Number::from(*value))),
    )
}

#[must_use]
pub fn json_f64() -> PureCodec<f64, JsonValue> {
    PureCodec::with_ast(
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
    )
}

/// Encode an exact decimal as a canonical JSON string. A JSON number may
/// already have lost precision before it reaches a codec, so it is rejected.
#[must_use]
pub fn json_decimal() -> PureCodec<Decimal, JsonValue> {
    PureCodec::with_ast(
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
    )
}

/// Persist a UTC epoch-millisecond value as a signed JSON integer.
#[must_use]
pub fn json_utc_millis() -> PureCodec<UtcMillis, JsonValue> {
    let integer = json_i64();
    PureCodec::with_ast(
        integer.ast().clone(),
        move |encoded| integer.decode_ref(encoded).map(UtcMillis::new),
        |value| Ok(JsonValue::Number(serde_json::Number::from(value.value()))),
    )
}

/// Persist a domain-checked stable key using its versioned byte encoding.
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
}

#[must_use]
pub fn json_nullable<A: Send + 'static>(
    value: PureCodec<A, JsonValue>,
) -> PureCodec<Option<A>, JsonValue> {
    let ast = SchemaAst::new(SchemaShape::Union(vec![
        SchemaAst::primitive(PrimitiveShape::Null),
        value.ast().clone(),
    ]));
    let decode = value.clone();
    PureCodec::with_ast(
        ast,
        move |encoded| {
            if matches!(encoded, JsonValue::Null) {
                Ok(None)
            } else {
                decode.decode_ref(encoded).map(Some)
            }
        },
        move |decoded| match decoded {
            None => Ok(JsonValue::Null),
            Some(item) => value.encode_ref(item),
        },
    )
}

/// A collection limit is a caller-supplied policy. `None` deliberately allows
/// the full already-decoded JSON array; no hidden capacity is selected.
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
    PureCodec::with_ast(
        ast,
        move |encoded| {
            let JsonValue::Array(items) = encoded else {
                return Err(mismatch(ValueKind::Sequence, encoded));
            };
            if maximum.is_some_and(|limit| items.len() > limit.get()) {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    decode
                        .decode_ref(item)
                        .map_err(|issue| issue.at(IssueSegment::Index(index)))
                })
                .collect()
        },
        move |decoded: &Vec<A>| {
            if maximum.is_some_and(|limit| decoded.len() > limit.get()) {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: super::SchemaResource::Width,
                }));
            }
            decoded
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    element
                        .encode_ref(item)
                        .map_err(|issue| issue.at(IssueSegment::Index(index)))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(JsonValue::Array)
        },
    )
}

/// A JSON pair has exactly two elements. Element issues retain their indices.
#[must_use]
pub fn json_tuple2<A: Send + 'static, B: Send + 'static>(
    first: PureCodec<A, JsonValue>,
    second: PureCodec<B, JsonValue>,
) -> PureCodec<(A, B), JsonValue> {
    let ast = SchemaAst::new(SchemaShape::Tuple(vec![
        first.ast().clone(),
        second.ast().clone(),
    ]));
    let decode_first = first.clone();
    let decode_second = second.clone();
    PureCodec::with_ast(
        ast,
        move |encoded| {
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
                .decode_ref(left)
                .map_err(|issue| issue.at(IssueSegment::Index(0)))?;
            let right = decode_second
                .decode_ref(right)
                .map_err(|issue| issue.at(IssueSegment::Index(1)))?;
            Ok((left, right))
        },
        move |(left, right)| {
            let left = first
                .encode_ref(left)
                .map_err(|issue| issue.at(IssueSegment::Index(0)))?;
            let right = second
                .encode_ref(right)
                .map_err(|issue| issue.at(IssueSegment::Index(1)))?;
            Ok(JsonValue::Array(vec![left, right]))
        },
    )
}

/// JSON byte arrays use unsigned integer elements and reject out-of-range
/// bytes rather than truncating them. Binary framing can use a different codec.
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
    PureCodec::with_ast(
        ast,
        move |encoded| {
            let JsonValue::Array(items) = encoded else {
                return Err(mismatch(ValueKind::Sequence, encoded));
            };
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
        move |decoded: &Vec<u8>| {
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
    )
}

/// Derive a literal codec from an explicit application-owned JSON value.
#[must_use]
pub fn json_literal(value: Arc<JsonValue>) -> PureCodec<(), JsonValue> {
    let ast = SchemaAst::new(SchemaShape::Literal(Arc::clone(&value)));
    let decode = Arc::clone(&value);
    PureCodec::with_ast(
        ast,
        move |input| {
            if input == decode.as_ref() {
                Ok(())
            } else {
                Err(SchemaIssue::new(SchemaIssueKind::LiteralMismatch))
            }
        },
        move |_| Ok(value.as_ref().clone()),
    )
}
