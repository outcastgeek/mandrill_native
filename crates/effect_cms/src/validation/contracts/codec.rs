//! Codec constructors shared by the builtin document contracts.
//!
//! Each constructor produces a borrowed JSON-tree codec whose accepted shape is
//! explicit: strings measure Unicode scalar length, enums match declared scalar
//! values, integers require an integral value inside an inclusive bound, maps
//! route declared keys and reject the rest, and records combine declared field
//! codecs with a required-presence check. Construction failures are reported to
//! the caller instead of producing a permissive codec.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use effect::{
    IssueSegment, JsonExactNumber, JsonValue, LiteralEnumValue, MultiplePatternPolicy,
    NumericConstraint, ObjectConstraints, ParseContext, ParsePolicy, PureCodec, SchemaAst,
    SchemaErrorMode, SchemaIssue, SchemaIssueKind, SchemaResource, StringConstraints,
    UnknownLiteralPolicy, ValueKind, codec, json_all_of, json_array, json_bool, json_exact_number,
    json_literal_enum, json_nullable, json_object_constraints, json_string,
    json_string_constraints, value_kind,
};

/// One borrowed JSON-tree codec used by the builtin contracts.
pub(super) type ContractCodec = PureCodec<JsonValue, JsonValue>;

/// Whether a declared record field must be present.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Presence {
    /// The field must be present.
    Required,
    /// The field may be absent.
    Optional,
    /// The field may be absent; an absent value denotes explicit null.
    DefaultNull,
}

/// One declared record field and its presence rule.
#[derive(Clone)]
pub(super) struct FieldSpec {
    name: &'static str,
    codec: ContractCodec,
    presence: Presence,
}

/// Declare a required field.
pub(super) fn required(name: &'static str, codec: ContractCodec) -> FieldSpec {
    FieldSpec {
        name,
        codec,
        presence: Presence::Required,
    }
}

/// Declare an optional field.
pub(super) fn optional(name: &'static str, codec: ContractCodec) -> FieldSpec {
    FieldSpec {
        name,
        codec,
        presence: Presence::Optional,
    }
}

/// Declare an optional field whose absent value denotes explicit null.
pub(super) fn default_null(name: &'static str, codec: ContractCodec) -> FieldSpec {
    FieldSpec {
        name,
        codec,
        presence: Presence::DefaultNull,
    }
}

/// Accept any JSON value, including null.
pub(super) fn any() -> ContractCodec {
    codec! {
        pure;
        decode: |value: &JsonValue| Ok(value.clone()),
        encode: |value: &JsonValue| Ok(value.clone()),
    }
}

/// Accept a string of at least `minimum` Unicode scalars with no upper bound.
pub(super) fn string_min(minimum: usize) -> Result<ContractCodec, SchemaIssue> {
    let codec = json_string_constraints(StringConstraints {
        minimum,
        maximum: usize::MAX,
        pattern: None,
        uuid_v4: false,
    })?;
    Ok(codec! {
        transform_pure;
        source: codec,
        decode: |value| Ok(JsonValue::String(value)),
        encode: |value| {
            if let JsonValue::String(text) = value {
                Ok(text.clone())
            } else {
                Err(mismatch(ValueKind::String, value))
            }
        },
    })
}

/// Accept a boolean.
pub(super) fn boolean() -> ContractCodec {
    codec! {
        transform_pure;
        source: json_bool(),
        decode: |value| Ok(JsonValue::Bool(value)),
        encode: |value| {
            if let JsonValue::Bool(flag) = value {
                Ok(*flag)
            } else {
                Err(mismatch(ValueKind::Boolean, value))
            }
        },
    }
}

/// Accept exactly one of the declared string values.
pub(super) fn one_of(allowed: &'static [&'static str]) -> Result<ContractCodec, SchemaIssue> {
    let literals: Vec<JsonValue> = allowed
        .iter()
        .map(|value| JsonValue::String((*value).to_owned()))
        .collect();
    let decoded = literals.clone();
    let encoded = literals;
    Ok(codec! {
        transform_pure;
        source: json_literal_enum(decoded.clone(), UnknownLiteralPolicy::Reject)?,
        decode: move |value| match value {
            LiteralEnumValue::Known(index) => decoded
                .get(index)
                .cloned()
                .ok_or(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)),
            LiteralEnumValue::Unknown(_) => {
                Err(SchemaIssue::new(SchemaIssueKind::LiteralMismatch))
            }
        },
        encode: move |value| {
            if let JsonValue::String(text) = value {
                encoded
                    .iter()
                    .position(|literal| literal.as_str() == Some(text.as_str()))
                    .map(LiteralEnumValue::Known)
                    .ok_or(SchemaIssue::new(SchemaIssueKind::LiteralMismatch))
            } else {
                Err(mismatch(ValueKind::String, value))
            }
        },
    })
}

/// Accept an integral JSON number no smaller than `minimum`.
pub(super) fn integer_min(minimum: i64) -> Result<ContractCodec, SchemaIssue> {
    let digits = NonZeroUsize::MAX;
    let bound = JsonExactNumber::parse(&minimum.to_string(), digits)?;
    number(
        vec![
            NumericConstraint::Integer,
            NumericConstraint::Minimum(bound),
        ],
        digits,
    )
}

/// Accept a JSON number inside the inclusive integer bounds.
pub(super) fn number_between(minimum: i64, maximum: i64) -> Result<ContractCodec, SchemaIssue> {
    let digits = NonZeroUsize::MAX;
    let lower = JsonExactNumber::parse(&minimum.to_string(), digits)?;
    let upper = JsonExactNumber::parse(&maximum.to_string(), digits)?;
    number(
        vec![
            NumericConstraint::Minimum(lower),
            NumericConstraint::Maximum(upper),
        ],
        digits,
    )
}

fn number(
    constraints: Vec<NumericConstraint>,
    digits: NonZeroUsize,
) -> Result<ContractCodec, SchemaIssue> {
    // The documents impose no digit limit, so parsing uses the largest
    // representable count rather than adding a contract constraint.
    let codec = json_exact_number(constraints, digits)?;
    Ok(codec! {
        transform_pure;
        source: codec,
        decode: |value| {
            value
                .as_str()
                .parse::<serde_json::Number>()
                .map(JsonValue::Number)
                .map_err(|_| SchemaIssue::new(SchemaIssueKind::OutOfRange))
        },
        encode: move |value| {
            if let JsonValue::Number(number) = value {
                JsonExactNumber::parse(&number.to_string(), digits)
            } else {
                Err(mismatch(ValueKind::Number, value))
            }
        },
    })
}

/// Accept null or the supplied value codec.
pub(super) fn nullable(codec: ContractCodec) -> ContractCodec {
    codec! {
        transform_pure;
        source: json_nullable(codec),
        decode: |value| match value {
            Some(value) => Ok(value),
            None => Ok(JsonValue::Null),
        },
        encode: |value| {
            if let JsonValue::Null = value {
                Ok(None)
            } else {
                Ok(Some(value.clone()))
            }
        },
    }
}

/// Accept a JSON array whose elements satisfy the supplied codec.
pub(super) fn array(policy: ParsePolicy, element: ContractCodec) -> ContractCodec {
    codec! {
        transform_pure;
        source: json_array(element, Some(policy.limits.collection)),
        decode: |items| Ok(JsonValue::Array(items)),
        encode: |value| {
            if let JsonValue::Array(items) = value {
                Ok(items.clone())
            } else {
                Err(mismatch(ValueKind::Sequence, value))
            }
        },
    }
}

/// Accept an object whose property values all satisfy `value`.
pub(super) fn nested_map(
    policy: ParsePolicy,
    value: ContractCodec,
) -> Result<ContractCodec, SchemaIssue> {
    map(policy, BTreeMap::new(), Some(value))
}

/// Accept an object with string values keyed by any property name.
pub(super) fn string_map(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    nested_map(policy, string_min(0)?)
}

/// Accept null or an object with string values keyed by any property name.
pub(super) fn optional_string_map(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    Ok(nullable(string_map(policy)?))
}

/// Accept null or an object whose property values are unconstrained.
pub(super) fn loose_map(policy: ParsePolicy) -> Result<ContractCodec, SchemaIssue> {
    Ok(nullable(nested_map(policy, any())?))
}

/// Accept an object whose property keys must be declared and whose values all
/// satisfy the declared codec.
pub(super) fn declared_map(
    policy: ParsePolicy,
    keys: &'static [&'static str],
    value: ContractCodec,
) -> Result<ContractCodec, SchemaIssue> {
    let mut declared = BTreeMap::new();
    for key in keys {
        declared.insert((*key).to_owned(), value.clone());
    }
    map(policy, declared, None)
}

fn map(
    policy: ParsePolicy,
    declared: BTreeMap<String, ContractCodec>,
    additional: Option<ContractCodec>,
) -> Result<ContractCodec, SchemaIssue> {
    let constraints = ObjectConstraints {
        names: json_string(),
        declared,
        patterns: Vec::new(),
        additional,
        minimum: 0,
        maximum: policy.limits.collection.get(),
        multiple_patterns: MultiplePatternPolicy::Reject,
    };
    let codec = json_object_constraints(constraints, policy)?;
    Ok(codec! {
        transform_pure;
        source: codec,
        decode: |values: BTreeMap<String, JsonValue>| {
            let fields: serde_json::Map<String, JsonValue> = values.into_iter().collect();
            Ok(JsonValue::Object(fields))
        },
        encode: |value: &JsonValue| -> Result<BTreeMap<String, JsonValue>, SchemaIssue> {
            if let JsonValue::Object(fields) = value {
                Ok(fields
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect())
            } else {
                Err(mismatch(ValueKind::Record, value))
            }
        },
    })
}

/// Accept a closed record whose declared fields satisfy their codecs and whose
/// required fields are present.
pub(super) fn object(
    fields: Vec<FieldSpec>,
    policy: ParsePolicy,
) -> Result<ContractCodec, SchemaIssue> {
    let mut required_names: Vec<&'static str> = Vec::new();
    let mut declared: BTreeMap<String, ContractCodec> = BTreeMap::new();
    for field in fields {
        if matches!(field.presence, Presence::Required) {
            required_names.push(field.name);
        }
        if declared
            .insert(field.name.to_owned(), field.codec)
            .is_some()
        {
            return Err(SchemaIssue::new(SchemaIssueKind::DuplicateField(
                Arc::from(field.name),
            )));
        }
    }
    let values = map(policy, declared, None)?;
    if required_names.is_empty() {
        return Ok(values);
    }
    let decode_names = required_names.clone();
    let encode_names = required_names;
    let presence: ContractCodec = codec! {
        contextual_pure;
        ast: SchemaAst::opaque(),
        decode: move |value: &JsonValue, context: Option<&ParseContext>| {
            required_presence(value, &decode_names, context, policy)
        },
        encode: move |value: &JsonValue, context: Option<&ParseContext>| {
            required_presence(value, &encode_names, context, policy)
        },
    };
    json_all_of(vec![presence, values], 1, policy)
}

fn required_presence(
    value: &JsonValue,
    required_names: &[&'static str],
    context: Option<&ParseContext>,
    policy: ParsePolicy,
) -> Result<JsonValue, SchemaIssue> {
    let active = context.map_or(policy, |context| context.policy().parse);
    let budget = context.map(|context| context.issue_budget());
    let JsonValue::Object(object) = value else {
        return Err(mismatch(ValueKind::Record, value));
    };
    let mut issues: Vec<SchemaIssue> = Vec::new();
    let mut composite_charged = false;
    for name in required_names {
        if object.contains_key(*name) {
            continue;
        }
        let segment: Arc<str> = Arc::from(*name);
        let missing = SchemaIssue::new(SchemaIssueKind::MissingField(Arc::clone(&segment)))
            .at(IssueSegment::Field(segment));
        if active.errors == SchemaErrorMode::First {
            if let Some(budget) = budget.as_ref() {
                let before = budget.used();
                budget.record(before, &missing)?;
            }
            return Err(missing);
        }
        if let Some(budget) = budget.as_ref() {
            let before = budget.used();
            budget.record(before, &missing)?;
            if !issues.is_empty() && !composite_charged {
                let before = budget.used();
                budget.record(before, &SchemaIssue::new(SchemaIssueKind::Multiple))?;
                composite_charged = true;
            }
        } else {
            let count = issues.len().saturating_add(1);
            let nodes = count.saturating_add(usize::from(count > 1));
            if nodes > active.limits.issues.get() {
                return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: SchemaResource::Issues,
                }));
            }
        }
        issues.push(missing);
    }
    match issues.len() {
        0 => Ok(value.clone()),
        1 => Err(issues.remove(0)),
        _ => Err(SchemaIssue::new(SchemaIssueKind::Multiple).with_children(issues)),
    }
}

fn mismatch(expected: ValueKind, value: &JsonValue) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::TypeMismatch {
        expected,
        actual: value_kind(value),
    })
}
