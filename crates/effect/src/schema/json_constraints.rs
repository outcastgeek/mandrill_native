//! Prepared JSON constraints with caller-owned policies.

use super::{
    IssueSegment, JsonValue, ParsePolicy, PureCodec, SchemaAst, SchemaIssue, SchemaIssueKind,
    SchemaResource, SchemaShape, StringLengthUnit, ValueKind, value_kind,
};
use crate::codec;
use regex_automata::nfa::thompson::{self, pikevm::PikeVM};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
/// Explicit limits for compiling and searching one checked regular expression.
#[derive(Clone, Copy, Debug)]
pub struct PatternPolicy {
    /// Maximum Thompson NFA compilation size in bytes.
    pub compiled_bytes: NonZeroUsize,
    /// Maximum regex syntax nesting; must fit the compiler's u32 nesting limit.
    pub syntax_depth: NonZeroUsize,
    /// Maximum conservative search demand: UTF-8 input bytes plus one, multiplied by NFA states.
    pub steps: NonZeroUsize,
}
/// A prepared bounded PikeVM pattern; clones share immutable compiled state.
///
/// Each search has a fresh matcher cache. In configured codecs, demand is also
/// charged to the invocation's cumulative pattern budget. Search follows regex
/// semantics: use anchors in the expression when a full-string match is required.
#[derive(Clone)]
pub struct CheckedPattern {
    source: Arc<str>,
    vm: Arc<PikeVM>,
    policy: PatternPolicy,
}
impl CheckedPattern {
    /// Compile under the supplied syntax and size bounds; invalid or over-budget patterns return `InvalidDefinition`.
    pub fn new(source: impl Into<Arc<str>>, policy: PatternPolicy) -> Result<Self, SchemaIssue> {
        let source = source.into();
        let depth = u32::try_from(policy.syntax_depth.get()).map_err(|_| invalid())?;
        let vm = PikeVM::builder()
            .syntax(regex_automata::util::syntax::Config::new().nest_limit(depth))
            .thompson(thompson::Config::new().nfa_size_limit(Some(policy.compiled_bytes.get())))
            .build(&source)
            .map_err(|_| invalid())?;
        Ok(Self {
            source,
            vm: Arc::new(vm),
            policy,
        })
    }
    /// Borrow the original pattern source used for introspection.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
    /// Search `text` using this pattern's explicit per-search budget.
    ///
    /// Returns `PatternBudget` before running an excessive or overflowing search.
    /// This standalone method has no shared configured invocation budget.
    pub fn matches(&self, text: &str) -> Result<bool, SchemaIssue> {
        // Thompson simulation visits each state at most once per input position.
        // The extra position is the end-of-input transition, not a policy default.
        self.matches_in(text, None)
    }
    fn matches_in(
        &self,
        text: &str,
        context: Option<&super::ParseContext>,
    ) -> Result<bool, SchemaIssue> {
        let demand = text
            .len()
            .checked_add(1)
            .and_then(|len| len.checked_mul(self.vm.get_nfa().states().len()));
        if demand.is_none_or(|demand| demand > self.policy.steps.get()) {
            return Err(SchemaIssue::new(SchemaIssueKind::PatternBudget));
        }
        if let Some(context) = context {
            context.charge_pattern(
                demand.ok_or_else(|| SchemaIssue::new(SchemaIssueKind::PatternBudget))?,
            )?;
        }
        let mut cache = self.vm.create_cache();
        Ok(self.vm.is_match(&mut cache, text))
    }
}
fn invalid() -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
}
fn constraint(name: &'static str, operand: impl Into<Arc<str>>) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::Constraint {
        name: name.into(),
        operand: operand.into(),
    })
}
/// Inclusive Unicode scalar-length bounds, an optional pattern and optional UUID v4 validation.
#[derive(Clone)]
pub struct StringConstraints {
    /// Minimum Unicode scalar count, including zero for an allowed empty string.
    pub minimum: usize,
    /// Maximum Unicode scalar count, independent of UTF-8 byte length.
    pub maximum: usize,
    /// Optional bounded regex search, using anchors when full-string matching is required.
    pub pattern: Option<CheckedPattern>,
    /// Require RFC UUID text layout, version-4 nibble and RFC variant bits when true.
    pub uuid_v4: bool,
}
impl StringConstraints {
    fn validate_in(
        &self,
        value: &str,
        context: Option<&super::ParseContext>,
    ) -> Result<(), SchemaIssue> {
        if let Some(context) = context {
            let limit = context.policy().string_length.get();
            let observed = value.chars().count();
            if observed > limit {
                return Err(SchemaIssue::new(SchemaIssueKind::ResourceDemand {
                    resource: SchemaResource::StringLength,
                    limit,
                    observed,
                }));
            }
        }
        let length = value.chars().count();
        if length < self.minimum || length > self.maximum {
            return Err(constraint(
                "stringLength",
                format!("{}..={}", self.minimum, self.maximum),
            ));
        }
        if let Some(pattern) = &self.pattern
            && !pattern.matches_in(value, context)?
        {
            return Err(SchemaIssue::new(SchemaIssueKind::PatternMismatch));
        }
        if self.uuid_v4 {
            // RFC 9562 UUID textual layout, version 4 nibble and RFC variant.
            let mut nibbles = Vec::new();
            for (index, byte) in value.bytes().enumerate() {
                if matches!(index, 8 | 13 | 18 | 23) {
                    if byte != b'-' {
                        return Err(constraint("uuid", "v4"));
                    }
                } else if let Some(nibble) = char::from(byte).to_digit(16) {
                    nibbles.push(nibble);
                } else {
                    return Err(constraint("uuid", "v4"));
                }
            }
            if value.len() != 36
                || nibbles.get(12) != Some(&4)
                || !nibbles
                    .get(16)
                    .is_some_and(|nibble| matches!(nibble, 8..=11))
            {
                return Err(constraint("uuid", "v4"));
            }
        }
        Ok(())
    }
}
/// Build a string codec with independent validation, direct JSON writing and equality.
///
/// Rejects reversed length bounds at construction. All directions apply Unicode
/// length, pattern and UUID checks; configured decode/encode/validation also use
/// the active string and cumulative pattern budgets. Constraint failures retain
/// the violated predicate; resource exhaustion is not treated as a mismatch.
pub fn json_string_constraints(
    constraints: StringConstraints,
) -> Result<PureCodec<String, JsonValue>, SchemaIssue> {
    if constraints.minimum > constraints.maximum {
        return Err(invalid());
    }
    let source = super::json_string();
    let mut ast = SchemaAst::new(SchemaShape::StringLength {
        source: source.ast().clone(),
        minimum: constraints.minimum,
        maximum: constraints.maximum,
        unit: StringLengthUnit::UnicodeScalars,
    });
    if constraints.pattern.is_some() || constraints.uuid_v4 {
        ast = ast.derive(SchemaShape::Checked {
            source: ast.clone(),
            name: "checked_string".into(),
        });
    }
    let decode = constraints.clone();
    let validate = constraints.clone();
    let writer_constraints = constraints.clone();
    Ok(PureCodec::with_context_ast(
        ast,
        move |value, context| {
            let value =
                source.decode_in_ref(value, super::parse_context::descend(context)?.as_ref())?;
            decode.validate_in(&value, context)?;
            Ok(value)
        },
        move |value: &String, context| {
            constraints.validate_in(value, context)?;
            Ok(JsonValue::String(value.clone()))
        },
    )
    .with_context_type_validator(move |value, context| validate.validate_in(value, context))
    .with_writer(move |value, writer| {
        writer_constraints.validate_in(value, None)?;
        writer.string(value)
    })
    .with_equivalence(|left, right| left == right))
}
/// Whether a scalar literal enum rejects or retains values outside its declared set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownLiteralPolicy {
    /// Reject values not equal to any declared literal.
    Reject,
    /// Retain unknown literals of the same scalar kind as the declared set.
    Preserve,
}
/// A declared literal index or a retained unknown scalar from [`json_literal_enum`].
#[derive(Clone, Debug, PartialEq)]
pub enum LiteralEnumValue {
    /// Zero-based index into the original declared literal list.
    Known(usize),
    /// An unknown scalar; valid only under Preserve and when its kind agrees with the declared set.
    Unknown(JsonValue),
}
fn literal_kind(value: &JsonValue) -> ValueKind {
    if value.is_number() {
        ValueKind::Number
    } else {
        value_kind(value)
    }
}
fn literal_equal(left: &JsonValue, right: &JsonValue) -> bool {
    match (left.as_number(), right.as_number()) {
        (Some(left), Some(right)) => {
            let left = left.to_string();
            let right = right.to_string();
            let limit = std::num::NonZeroUsize::new(left.len().max(right.len()))
                .unwrap_or(std::num::NonZeroUsize::MIN);
            match (
                super::JsonExactNumber::parse(&left, limit),
                super::JsonExactNumber::parse(&right, limit),
            ) {
                (Ok(left), Ok(right)) => left.compare(&right) == std::cmp::Ordering::Equal,
                _ => false,
            }
        }
        _ => match (left, right) {
            (JsonValue::Array(left), JsonValue::Array(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right)
                        .all(|(left, right)| literal_equal(left, right))
            }
            (JsonValue::Object(left), JsonValue::Object(right)) => {
                left.len() == right.len()
                    && left.iter().all(|(name, value)| {
                        right
                            .get(name)
                            .is_some_and(|other| literal_equal(value, other))
                    })
            }
            _ => left == right,
        },
    }
}
fn enum_value<'a>(
    value: &'a LiteralEnumValue,
    literals: &'a [JsonValue],
    unknown: UnknownLiteralPolicy,
    kind: &ValueKind,
) -> Result<&'a JsonValue, SchemaIssue> {
    match value {
        LiteralEnumValue::Known(index) => literals.get(*index).ok_or_else(invalid),
        LiteralEnumValue::Unknown(value)
            if unknown == UnknownLiteralPolicy::Preserve
                && literal_kind(value) == *kind
                && !literals.iter().any(|literal| literal_equal(literal, value)) =>
        {
            Ok(value)
        }
        LiteralEnumValue::Unknown(_) => Err(SchemaIssue::new(SchemaIssueKind::LiteralMismatch)),
    }
}
/// Build a homogeneous scalar enum with checked indexes and independent witnesses.
///
/// Rejects an empty list, containers, mixed scalar kinds and numerically equivalent
/// duplicates such as `1` and `1.0`. Encode/validation reject out-of-range Known
/// indexes and Unknown values that contradict the selected policy.
pub fn json_literal_enum(
    literals: Vec<JsonValue>,
    unknown: UnknownLiteralPolicy,
) -> Result<PureCodec<LiteralEnumValue, JsonValue>, SchemaIssue> {
    let Some(first) = literals.first() else {
        return Err(invalid());
    };
    let kind = literal_kind(first);
    if matches!(kind, ValueKind::Sequence | ValueKind::Record)
        || literals.iter().any(|value| literal_kind(value) != kind)
    {
        return Err(invalid());
    }
    for (index, value) in literals.iter().enumerate() {
        if literals
            .iter()
            .take(index)
            .any(|candidate| literal_equal(candidate, value))
        {
            return Err(invalid());
        }
    }
    let ast = if unknown == UnknownLiteralPolicy::Reject {
        SchemaAst::new(SchemaShape::Union(
            literals
                .iter()
                .map(|value| SchemaAst::new(SchemaShape::Literal(Arc::new(value.clone()))))
                .collect(),
        ))
    } else {
        SchemaAst::opaque()
    };
    let literals: Arc<[JsonValue]> = literals.into();
    let decode = Arc::clone(&literals);
    let decode_kind = kind.clone();
    let validate_literals = Arc::clone(&literals);
    let validate_kind = kind.clone();
    let writer_literals = Arc::clone(&literals);
    let writer_kind = kind.clone();
    Ok(codec! { pure; ast: ast, decode: move |value| {
        if let Some(index) = decode
            .iter()
            .position(|literal| literal_equal(literal, value))
        {
            Ok(LiteralEnumValue::Known(index))
        } else if unknown == UnknownLiteralPolicy::Preserve
            && literal_kind(value) == decode_kind
        {
            Ok(LiteralEnumValue::Unknown(value.clone()))
        } else {
            Err(SchemaIssue::new(SchemaIssueKind::LiteralMismatch))
        }
    }, encode: move |value| match value {
        LiteralEnumValue::Known(index) => literals.get(*index).cloned().ok_or_else(invalid),
        LiteralEnumValue::Unknown(value)
            if unknown == UnknownLiteralPolicy::Preserve
                && literal_kind(value) == kind
                && !literals.iter().any(|literal| literal_equal(literal, value)) =>
        {
            Ok(value.clone())
        }
        LiteralEnumValue::Unknown(_) => Err(SchemaIssue::new(SchemaIssueKind::LiteralMismatch)),
    }, }
    .with_type_validator(move |value| {
        enum_value(value, &validate_literals, unknown, &validate_kind).map(|_| ())
    })
    .with_writer(move |value, writer| {
        writer.tree(enum_value(value, &writer_literals, unknown, &writer_kind)?)
    }))
}
pub(crate) fn resource_failure(issue: &SchemaIssue) -> bool {
    matches!(
        issue.kind(),
        SchemaIssueKind::LimitExceeded { .. }
            | SchemaIssueKind::ResourceDemand { .. }
            | SchemaIssueKind::PatternBudget
    ) || issue.children().iter().any(resource_failure)
}
/// Retain successful alternative values and bounded ordinary issues, while
/// propagating resource exhaustion immediately instead of trying later branches.
fn branch_matches<A: Send + 'static>(
    codecs: &[PureCodec<A, JsonValue>],
    value: &JsonValue,
    policy: ParsePolicy,
    context: Option<&super::ParseContext>,
) -> Result<Vec<(usize, A)>, SchemaIssue> {
    let policy = super::parse_context::policy(context, policy);
    super::internal::parser::check_limits(value, &policy.limits)?;
    let budget = super::parse_context::budget(context, policy);
    let initial = budget.used();
    let mut matches = Vec::new();
    let mut issues = Vec::new();
    for (index, codec) in codecs.iter().enumerate() {
        budget.available()?;
        let before = budget.used();
        let child = super::parse_context::child(context, IssueSegment::Alternative(index))?;
        match codec.decode_in_ref(value, child.as_ref()) {
            Ok(value) => matches.push((index, value)),
            Err(issue) if resource_failure(&issue) => {
                return Err(issue.at(IssueSegment::Alternative(index)));
            }
            Err(issue) => {
                let issue = issue.at(IssueSegment::Alternative(index));
                budget.record(before, &issue)?;
                issues.push(issue);
            }
        }
    }
    if matches.is_empty() {
        let issue = SchemaIssue::new(SchemaIssueKind::NoAlternative).with_children(issues);
        budget.record(initial, &issue)?;
        Err(issue)
    } else {
        Ok(matches)
    }
}
fn typed_matches<A: Send + 'static>(
    codecs: &[PureCodec<A, JsonValue>],
    value: &A,
    policy: ParsePolicy,
    context: Option<&super::ParseContext>,
) -> Result<Vec<usize>, SchemaIssue> {
    let policy = super::parse_context::policy(context, policy);
    let budget = super::parse_context::budget(context, policy);
    let initial = budget.used();
    let mut matches = Vec::new();
    let mut issues = Vec::new();
    for (index, codec) in codecs.iter().enumerate() {
        budget.available()?;
        let before = budget.used();
        let child = super::parse_context::child(context, IssueSegment::Alternative(index))?;
        match codec.validate_in_ref(value, child.as_ref()) {
            Ok(()) => matches.push(index),
            Err(issue) if resource_failure(&issue) => {
                return Err(issue.at(IssueSegment::Alternative(index)));
            }
            Err(issue) => {
                let issue = issue.at(IssueSegment::Alternative(index));
                budget.record(before, &issue)?;
                issues.push(issue);
            }
        }
    }
    if matches.is_empty() {
        let issue = SchemaIssue::new(SchemaIssueKind::NoAlternative).with_children(issues);
        budget.record(initial, &issue)?;
        Err(issue)
    } else {
        Ok(matches)
    }
}
/// Require exactly one successful alternative and return its owned decoded value.
///
/// Zero matches produce `NoAlternative`; multiple matches produce
/// `AmbiguousAlternatives`. Resource failures stop evaluation immediately and
/// ordinary branch errors carry alternative paths within the issue budget.
/// `select_encode` chooses an alternative index. Independent type validation and
/// writing exist only if every branch supplies the required genuine witnesses;
/// those paths verify that exactly one typed branch matches the selected index.
pub fn json_one_of<A: Send + 'static>(
    codecs: Vec<PureCodec<A, JsonValue>>,
    select_encode: impl Fn(&A) -> usize + Send + Sync + 'static,
    policy: ParsePolicy,
) -> Result<PureCodec<A, JsonValue>, SchemaIssue> {
    if codecs.is_empty() || codecs.len() > policy.limits.collection.get() {
        return Err(invalid());
    }
    let ast = SchemaAst::new(SchemaShape::Checked {
        source: SchemaAst::new(SchemaShape::Union(
            codecs.iter().map(|c| c.ast().clone()).collect(),
        )),
        name: "oneOf".into(),
    });
    let codecs: Arc<[PureCodec<A, JsonValue>]> = codecs.into();
    let decode = Arc::clone(&codecs);
    let validate = Arc::clone(&codecs);
    let write = Arc::clone(&codecs);
    let has_validator = codecs.iter().all(PureCodec::has_type_validator);
    let has_writer = has_validator && codecs.iter().all(PureCodec::has_writer);
    let select = Arc::new(select_encode);
    let validate_select = Arc::clone(&select);
    let write_select = Arc::clone(&select);
    let result = PureCodec::with_context_ast(
        ast,
        move |value, context| {
            let mut matches = branch_matches(&decode, value, policy, context)?;
            if matches.len() != 1 {
                return Err(SchemaIssue::new(SchemaIssueKind::AmbiguousAlternatives {
                    matches: matches.iter().map(|(index, _)| *index).collect(),
                }));
            }
            matches.pop().map(|(_, value)| value).ok_or_else(invalid)
        },
        move |value, context| {
            let selected = select(value);
            let child = super::parse_context::child(context, IssueSegment::Alternative(selected))?;
            let encoded = codecs
                .get(selected)
                .ok_or_else(invalid)?
                .encode_in_ref(value, child.as_ref())?;
            let matches = branch_matches(&codecs, &encoded, policy, context)?;
            if matches.len() != 1 {
                return Err(SchemaIssue::new(SchemaIssueKind::AmbiguousAlternatives {
                    matches: matches.iter().map(|(index, _)| *index).collect(),
                }));
            }
            Ok(encoded)
        },
    );
    let result = if has_validator {
        result.with_context_type_validator(move |value, context| {
            let matches = typed_matches(&validate, value, policy, context)?;
            if matches.len() != 1 {
                return Err(SchemaIssue::new(SchemaIssueKind::AmbiguousAlternatives {
                    matches,
                }));
            }
            if matches.first().copied() != Some(validate_select(value)) {
                return Err(invalid());
            }
            Ok(())
        })
    } else {
        result
    };
    Ok(if has_writer {
        result.with_writer(move |value, writer| {
            let matches = typed_matches(&write, value, policy, None)?;
            if matches.len() != 1 {
                return Err(SchemaIssue::new(SchemaIssueKind::AmbiguousAlternatives {
                    matches,
                }));
            }
            let selected = write_select(value);
            if matches.first().copied() != Some(selected) {
                return Err(invalid());
            }
            write
                .get(selected)
                .ok_or_else(invalid)?
                .write_into(value, writer)
        })
    } else {
        result
    })
}
/// An immutable original JSON input and the owned results of every matching alternative.
///
/// Only [`json_any_of`] constructs this invariant. Encoding returns the retained
/// input without replaying decoders or choosing one branch; inspect results with
/// [`Self::matches`] and the original representation with [`Self::input`].
pub struct AnyOfValue<A> {
    input: JsonValue,
    matches: Vec<(usize, A)>,
}
impl<A> AnyOfValue<A> {
    /// Borrow the original representation retained by a successful anyOf decode.
    #[must_use]
    pub const fn input(&self) -> &JsonValue {
        &self.input
    }
    /// Borrow successful `(alternative_index, decoded_value)` pairs in declaration order.
    #[must_use]
    pub fn matches(&self) -> &[(usize, A)] {
        &self.matches
    }
}
/// Require at least one successful branch and retain all matches plus the original input.
///
/// No-match errors include bounded alternative failures; resource exhaustion stops
/// immediately. Encoding preserves the retained representation without decoder
/// replay. An independent validator requires all child validators; direct writing
/// uses the retained checked JSON input rather than child encoders.
pub fn json_any_of<A: Send + 'static>(
    codecs: Vec<PureCodec<A, JsonValue>>,
    policy: ParsePolicy,
) -> Result<PureCodec<AnyOfValue<A>, JsonValue>, SchemaIssue> {
    if codecs.is_empty() || codecs.len() > policy.limits.collection.get() {
        return Err(invalid());
    }
    let ast = SchemaAst::new(SchemaShape::Union(
        codecs.iter().map(|c| c.ast().clone()).collect(),
    ));
    let codecs: Arc<[PureCodec<A, JsonValue>]> = codecs.into();
    let decode = Arc::clone(&codecs);
    let validate = Arc::clone(&codecs);
    let write = Arc::clone(&codecs);
    let has_validator = codecs.iter().all(PureCodec::has_type_validator);
    let result = PureCodec::with_context_ast(
        ast,
        move |value: &JsonValue, context| {
            Ok(AnyOfValue {
                input: value.clone(),
                matches: branch_matches(&decode, value, policy, context)?,
            })
        },
        move |value: &AnyOfValue<A>, context| {
            let active = super::parse_context::policy(context, policy);
            super::internal::parser::check_limits(&value.input, &active.limits)?;
            Ok(value.input.clone())
        },
    );
    let result = if has_validator {
        result.with_context_type_validator(move |value: &AnyOfValue<A>, context| {
            validate_any(value, &validate, policy, context)
        })
    } else {
        result
    };
    // anyOf stores its original JSON as part of the typed value. This is its
    // explicit retained-input writer, never a fallback to a child encoder.
    Ok(if has_validator {
        result.with_writer(move |value, writer| {
            validate_any(value, &write, policy, None)?;
            writer.tree(&value.input)
        })
    } else {
        result
    })
}
fn validate_any<A: Send + 'static>(
    value: &AnyOfValue<A>,
    codecs: &[PureCodec<A, JsonValue>],
    policy: ParsePolicy,
    context: Option<&super::ParseContext>,
) -> Result<(), SchemaIssue> {
    let active = super::parse_context::policy(context, policy);
    super::internal::parser::check_limits(&value.input, &active.limits)?;
    if value.matches.is_empty() {
        return Err(invalid());
    }
    for (index, value) in &value.matches {
        let child = super::parse_context::child(context, IssueSegment::Alternative(*index))?;
        codecs
            .get(*index)
            .ok_or_else(invalid)?
            .validate_in_ref(value, child.as_ref())
            .map_err(|issue| issue.at(IssueSegment::Alternative(*index)))?;
    }
    Ok(())
}
fn merge(left: &mut JsonValue, right: JsonValue) -> Result<(), SchemaIssue> {
    match (left, right) {
        (JsonValue::Object(left), JsonValue::Object(right)) => {
            for (name, value) in right {
                match left.get_mut(&name) {
                    Some(previous) => merge(previous, value)
                        .map_err(|issue| issue.at(IssueSegment::Field(Arc::from(name.as_str()))))?,
                    None => {
                        left.insert(name, value);
                    }
                }
            }
            Ok(())
        }
        (left, right) if literal_equal(left, &right) => Ok(()),
        _ => Err(SchemaIssue::new(SchemaIssueKind::MergeConflict)),
    }
}
/// Require every branch to decode and return the branch indexed by `select_decode`.
///
/// Encoding merges branch outputs recursively: object fields combine, while
/// conflicting values yield `MergeConflict`. Rejects an invalid selected index or
/// excessive branch count. Type validation requires all child validators; direct
/// writing requires all child writers and merges their checked outputs under the
/// supplied policy without invoking ordinary encoders.
pub fn json_all_of<A: Send + 'static>(
    codecs: Vec<PureCodec<A, JsonValue>>,
    select_decode: usize,
    policy: ParsePolicy,
) -> Result<PureCodec<A, JsonValue>, SchemaIssue> {
    if codecs.get(select_decode).is_none() || codecs.len() > policy.limits.collection.get() {
        return Err(invalid());
    }
    let ast = SchemaAst::new(SchemaShape::Checked {
        source: SchemaAst::opaque(),
        name: "allOf".into(),
    });
    let codecs: Arc<[PureCodec<A, JsonValue>]> = codecs.into();
    let decode = Arc::clone(&codecs);
    let validate = Arc::clone(&codecs);
    let write = Arc::clone(&codecs);
    let has_validator = codecs.iter().all(PureCodec::has_type_validator);
    let has_writer = codecs.iter().all(PureCodec::has_writer);
    let result = PureCodec::with_context_ast(
        ast,
        move |value, context| {
            let active = super::parse_context::policy(context, policy);
            super::internal::parser::check_limits(value, &active.limits)?;
            let mut selected = None;
            for (index, codec) in decode.iter().enumerate() {
                let child = super::parse_context::child(context, IssueSegment::Alternative(index))?;
                let decoded = codec
                    .decode_in_ref(value, child.as_ref())
                    .map_err(|issue| issue.at(IssueSegment::Alternative(index)))?;
                if index == select_decode {
                    selected = Some(decoded);
                }
            }
            selected.ok_or_else(invalid)
        },
        move |value, context| {
            let active = super::parse_context::policy(context, policy);
            let mut encoded = None;
            for (index, codec) in codecs.iter().enumerate() {
                let child = super::parse_context::child(context, IssueSegment::Alternative(index))?;
                let next = codec
                    .encode_in_ref(value, child.as_ref())
                    .map_err(|issue| issue.at(IssueSegment::Alternative(index)))?;
                super::internal::parser::check_limits(&next, &active.limits)?;
                match &mut encoded {
                    Some(previous) => merge(previous, next)?,
                    None => encoded = Some(next),
                }
            }
            let encoded = encoded.ok_or_else(invalid)?;
            super::internal::parser::check_limits(&encoded, &active.limits)?;
            Ok(encoded)
        },
    );
    let result = if has_validator {
        result.with_context_type_validator(move |value, context| {
            for (index, codec) in validate.iter().enumerate() {
                let child = super::parse_context::child(context, IssueSegment::Alternative(index))?;
                codec
                    .validate_in_ref(value, child.as_ref())
                    .map_err(|issue| issue.at(IssueSegment::Alternative(index)))?;
            }
            Ok(())
        })
    } else {
        result
    };
    Ok(if has_writer {
        result.with_writer(move |value, writer| {
            let mut encoded = None;
            for (index, codec) in write.iter().enumerate() {
                let bytes = codec
                    .write(value, writer.policy())
                    .map_err(|issue| issue.at(IssueSegment::Alternative(index)))?;
                let next = super::internal::parser::parse_json_bytes(&bytes, policy)?;
                match &mut encoded {
                    Some(previous) => merge(previous, next)?,
                    None => encoded = Some(next),
                }
            }
            writer.tree(&encoded.ok_or_else(invalid)?)
        })
    } else {
        result
    })
}
/// Inclusive array-length bounds and optional JSON/type-side uniqueness checks.
#[derive(Clone, Copy, Debug)]
pub struct ArrayConstraints {
    /// Minimum number of array elements, inclusive.
    pub minimum: usize,
    /// Maximum number of array elements, inclusive and no greater than policy width.
    pub maximum: usize,
    /// Require unique JSON elements; type-side checks additionally need a semantic equality witness.
    pub unique: bool,
}
/// Constrain array width and optional uniqueness, using the caller's traversal policy.
///
/// Rejects reversed bounds or a maximum above policy width. JSON uniqueness uses
/// numeric-aware literal equality. Independent typed uniqueness and writing need
/// [`PureCodec::with_equivalence`] on the element codec; missing witnesses are
/// not replaced by encoding/decoding round trips.
pub fn json_array_constraints<A: Send + 'static>(
    element: PureCodec<A, JsonValue>,
    constraints: ArrayConstraints,
    policy: ParsePolicy,
) -> Result<PureCodec<Vec<A>, JsonValue>, SchemaIssue> {
    if constraints.minimum > constraints.maximum
        || constraints.maximum > policy.limits.collection.get()
    {
        return Err(invalid());
    }
    let equivalent = element.equivalence();
    let validation_equivalent = equivalent.as_ref().map(Arc::clone);
    let writer_equivalent = equivalent.as_ref().map(Arc::clone);
    let base = super::json_array(element, Some(policy.limits.collection));
    let decode = base.clone();
    let validate = base.clone();
    let write = base.clone();
    let has_validator = base.has_type_validator() && (!constraints.unique || equivalent.is_some());
    let has_writer = base.has_writer() && (!constraints.unique || equivalent.is_some());
    let result = PureCodec::with_context_ast(
        base.ast().derive(SchemaShape::Checked {
            source: base.ast().clone(),
            name: "array_constraints".into(),
        }),
        move |value, context| {
            let active = super::parse_context::policy(context, policy);
            super::internal::parser::check_limits(value, &active.limits)?;
            let JsonValue::Array(items) = value else {
                return Err(invalid());
            };
            check_array_length(items.len(), constraints, active)?;
            if constraints.unique {
                for (index, item) in items.iter().enumerate() {
                    if items
                        .iter()
                        .take(index)
                        .any(|previous| literal_equal(previous, item))
                    {
                        return Err(
                            constraint("uniqueItems", "true").at(IssueSegment::Index(index))
                        );
                    }
                }
            }
            decode.decode_in_ref(value, context)
        },
        move |value, context| {
            let active = super::parse_context::policy(context, policy);
            check_array_length(value.len(), constraints, active)?;
            let encoded = base.encode_in_ref(value, context)?;
            if constraints.unique {
                let JsonValue::Array(items) = &encoded else {
                    return Err(invalid());
                };
                for (index, item) in items.iter().enumerate() {
                    if items
                        .iter()
                        .take(index)
                        .any(|previous| literal_equal(previous, item))
                    {
                        return Err(
                            constraint("uniqueItems", "true").at(IssueSegment::Index(index))
                        );
                    }
                }
            }
            Ok(encoded)
        },
    );
    let result = if has_validator {
        result.with_context_type_validator(move |value: &Vec<A>, context| {
            check_array_length(
                value.len(),
                constraints,
                super::parse_context::policy(context, policy),
            )?;
            check_typed_unique(value, constraints, validation_equivalent.as_deref())?;
            validate.validate_in_ref(value, context)
        })
    } else {
        result
    };
    Ok(if has_writer {
        result.with_writer(move |value, writer| {
            check_array_length(value.len(), constraints, policy)?;
            check_typed_unique(value, constraints, writer_equivalent.as_deref())?;
            write.write_into(value, writer)
        })
    } else {
        result
    })
}
fn check_array_length(
    length: usize,
    constraints: ArrayConstraints,
    policy: ParsePolicy,
) -> Result<(), SchemaIssue> {
    if length > policy.limits.collection.get() {
        return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
            resource: SchemaResource::Width,
        }));
    }
    if length < constraints.minimum || length > constraints.maximum {
        return Err(constraint(
            "arrayLength",
            format!("{}..={}", constraints.minimum, constraints.maximum),
        ));
    }
    Ok(())
}
type SemanticEquality<A> = dyn Fn(&A, &A) -> bool + Send + Sync;
fn check_typed_unique<A>(
    values: &[A],
    constraints: ArrayConstraints,
    equivalent: Option<&SemanticEquality<A>>,
) -> Result<(), SchemaIssue> {
    if constraints.unique {
        let equivalent =
            equivalent.ok_or_else(|| SchemaIssue::new(SchemaIssueKind::MissingValidator))?;
        for (index, value) in values.iter().enumerate() {
            if values
                .iter()
                .take(index)
                .any(|previous| equivalent(previous, value))
            {
                return Err(constraint("uniqueItems", "true").at(IssueSegment::Index(index)));
            }
        }
    }
    Ok(())
}
/// A positional prefix codec and whether its encoded position may be absent.
pub struct PrefixElement<A> {
    /// Codec for this prefix position; all positions share the prefix payload type.
    pub codec: PureCodec<A, JsonValue>,
    /// Whether decoding an absent position is an error.
    pub required: bool,
}
/// Owned decoded prefix slots and homogeneous remaining elements.
pub struct PrefixValue<A, B> {
    /// One slot per declared prefix position; None denotes trailing absence, not JSON null.
    pub prefix: Vec<Option<A>>,
    /// Elements after the declared prefix, validated by the optional rest codec.
    pub rest: Vec<B>,
}
/// Describe positional prefix entries and an optional homogeneous rest codec.
///
/// Required positions cannot be absent. Encoding/validation reject holes followed
/// by present slots or rest elements; absence is supported only at the end. When
/// `rest` is None, extra elements are rejected. Independent validation and writing
/// are exposed only if each participating child has the corresponding witness.
pub fn json_prefix_rest<A: Send + 'static, B: Send + 'static>(
    prefix: Vec<PrefixElement<A>>,
    rest: Option<PureCodec<B, JsonValue>>,
    policy: ParsePolicy,
) -> Result<PureCodec<PrefixValue<A, B>, JsonValue>, SchemaIssue> {
    if prefix.len() > policy.limits.collection.get() {
        return Err(invalid());
    }
    let prefix: Arc<[PrefixElement<A>]> = prefix.into();
    let decode = Arc::clone(&prefix);
    let decode_rest = rest.clone();
    let validate_prefix = Arc::clone(&prefix);
    let validate_rest = rest.clone();
    let write_prefix = Arc::clone(&prefix);
    let write_rest = rest.clone();
    let has_validator = prefix.iter().all(|p| p.codec.has_type_validator())
        && rest.as_ref().is_none_or(PureCodec::has_type_validator);
    let has_writer = prefix.iter().all(|p| p.codec.has_writer())
        && rest.as_ref().is_none_or(PureCodec::has_writer);
    let result = PureCodec::with_context_ast(
        SchemaAst::opaque(),
        move |value, context| {
            let policy = super::parse_context::policy(context, policy);
            super::internal::parser::check_limits(value, &policy.limits)?;
            let JsonValue::Array(items) = value else {
                return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                    expected: ValueKind::Sequence,
                    actual: value_kind(value),
                }));
            };
            let mut values = Vec::new();
            for (index, element) in decode.iter().enumerate() {
                match items.get(index) {
                    Some(value) => values.push(Some(
                        element
                            .codec
                            .decode_in_ref(
                                value,
                                super::parse_context::child(context, IssueSegment::Index(index))?
                                    .as_ref(),
                            )
                            .map_err(|issue| issue.at(IssueSegment::Index(index)))?,
                    )),
                    None if !element.required => values.push(None),
                    None => {
                        return Err(constraint("requiredPrefix", index.to_string())
                            .at(IssueSegment::Index(index)));
                    }
                }
            }
            let mut remaining = Vec::new();
            for (index, value) in items.iter().enumerate().skip(decode.len()) {
                let codec = decode_rest
                    .as_ref()
                    .ok_or_else(|| constraint("additionalItems", "false"))?;
                remaining.push(
                    codec
                        .decode_in_ref(
                            value,
                            super::parse_context::child(context, IssueSegment::Index(index))?
                                .as_ref(),
                        )
                        .map_err(|issue| issue.at(IssueSegment::Index(index)))?,
                );
            }
            Ok(PrefixValue {
                prefix: values,
                rest: remaining,
            })
        },
        move |value: &PrefixValue<A, B>, context| {
            let policy = super::parse_context::policy(context, policy);
            if value.prefix.len() != prefix.len() {
                return Err(invalid());
            }
            let mut encoded = Vec::new();
            let mut absent = false;
            for (index, (value, element)) in value.prefix.iter().zip(prefix.iter()).enumerate() {
                match value {
                    Some(value) if !absent => encoded.push(
                        element
                            .codec
                            .encode_in_ref(
                                value,
                                super::parse_context::child(context, IssueSegment::Index(index))?
                                    .as_ref(),
                            )
                            .map_err(|issue| issue.at(IssueSegment::Index(index)))?,
                    ),
                    None if !element.required => absent = true,
                    _ => return Err(invalid()),
                }
            }
            if absent && !value.rest.is_empty() {
                return Err(invalid());
            }
            for (offset, value) in value.rest.iter().enumerate() {
                let index = prefix.len().saturating_add(offset);
                let codec = rest.as_ref().ok_or_else(invalid)?;
                encoded.push(codec.encode_in_ref(
                    value,
                    super::parse_context::child(context, IssueSegment::Index(index))?.as_ref(),
                )?);
            }
            let value = JsonValue::Array(encoded);
            super::internal::parser::check_limits(&value, &policy.limits)?;
            Ok(value)
        },
    );
    let result = if has_validator {
        result.with_context_type_validator(move |value, context| {
            validate_prefix_value(
                value,
                &validate_prefix,
                validate_rest.as_ref(),
                super::parse_context::policy(context, policy),
                context,
            )
        })
    } else {
        result
    };
    Ok(if has_writer {
        result.with_writer(move |value, writer| {
            check_prefix_layout(value, &write_prefix, write_rest.is_some(), policy)?;
            writer.begin_array()?;
            for (index, (value, element)) in
                value.prefix.iter().zip(write_prefix.iter()).enumerate()
            {
                if let Some(value) = value {
                    element
                        .codec
                        .write_into(value, writer)
                        .map_err(|issue| issue.at(IssueSegment::Index(index)))?;
                }
            }
            for (offset, value) in value.rest.iter().enumerate() {
                write_rest
                    .as_ref()
                    .ok_or_else(invalid)?
                    .write_into(value, writer)
                    .map_err(|issue| {
                        issue.at(IssueSegment::Index(
                            write_prefix.len().saturating_add(offset),
                        ))
                    })?;
            }
            writer.end_array()
        })
    } else {
        result
    })
}
fn check_prefix_layout<A, B>(
    value: &PrefixValue<A, B>,
    prefix: &[PrefixElement<A>],
    has_rest: bool,
    policy: ParsePolicy,
) -> Result<(), SchemaIssue> {
    if value.prefix.len() != prefix.len() {
        return Err(invalid());
    }
    let mut absent = false;
    let mut length = value.rest.len();
    for (index, (value, element)) in value.prefix.iter().zip(prefix).enumerate() {
        match value {
            Some(_) if !absent => length = length.saturating_add(1),
            None if !element.required => absent = true,
            _ => {
                return Err(
                    constraint("requiredPrefix", index.to_string()).at(IssueSegment::Index(index))
                );
            }
        }
    }
    if (!has_rest || absent) && !value.rest.is_empty() {
        return Err(constraint("additionalItems", "false"));
    }
    if length > policy.limits.collection.get() {
        return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
            resource: SchemaResource::Width,
        }));
    }
    Ok(())
}
fn validate_prefix_value<A: Send + 'static, B: Send + 'static>(
    value: &PrefixValue<A, B>,
    prefix: &[PrefixElement<A>],
    rest: Option<&PureCodec<B, JsonValue>>,
    policy: ParsePolicy,
    context: Option<&super::ParseContext>,
) -> Result<(), SchemaIssue> {
    check_prefix_layout(value, prefix, rest.is_some(), policy)?;
    for (index, (value, element)) in value.prefix.iter().zip(prefix).enumerate() {
        if let Some(value) = value {
            let child = super::parse_context::child(context, IssueSegment::Index(index))?;
            element
                .codec
                .validate_in_ref(value, child.as_ref())
                .map_err(|issue| issue.at(IssueSegment::Index(index)))?;
        }
    }
    for (offset, value) in value.rest.iter().enumerate() {
        let index = prefix.len().saturating_add(offset);
        let child = super::parse_context::child(context, IssueSegment::Index(index))?;
        rest.ok_or_else(invalid)?
            .validate_in_ref(value, child.as_ref())
            .map_err(|issue| issue.at(IssueSegment::Index(index)))?;
    }
    Ok(())
}
/// How to handle a property name matching more than one pattern-property route.
#[derive(Clone, Copy, Debug)]
pub enum MultiplePatternPolicy {
    /// Reject a property name matching multiple patterns, even if their values could agree.
    Reject,
    /// Run every matching route; their encoded or written values must merge without conflict.
    ValidateAll,
}
/// A checked name pattern and the value codec selected by its match.
pub struct PatternProperty<A> {
    /// Bounded regex selecting matching property names.
    pub pattern: CheckedPattern,
    /// Value codec applied when this property-name pattern matches.
    pub codec: PureCodec<A, JsonValue>,
}
/// Property-name validation, declared/pattern routes, additional values and width bounds.
///
/// Declared routes and matching patterns apply together. The additional route is
/// used only when neither matches. These declarations route present properties;
/// they do not impose required-field presence.
pub struct ObjectConstraints<A> {
    /// Codec validating each property name as a JSON string; typed directions require its independent validator.
    pub names: PureCodec<String, JsonValue>,
    /// Named value routes applied alongside any matching pattern routes.
    pub declared: BTreeMap<String, PureCodec<A, JsonValue>>,
    /// Pattern routes searched in declaration order, with shared configured search demand.
    pub patterns: Vec<PatternProperty<A>>,
    /// Fallback codec for otherwise unmatched names; None rejects them as unknown fields.
    pub additional: Option<PureCodec<A, JsonValue>>,
    /// Minimum number of present object properties, inclusive.
    pub minimum: usize,
    /// Maximum number of present object properties, inclusive and bounded by policy width.
    pub maximum: usize,
    /// Whether multiple matching pattern routes are rejected or all checked.
    pub multiple_patterns: MultiplePatternPolicy,
}
impl<A: Send + 'static> ObjectConstraints<A> {
    fn select(
        &self,
        name: &str,
        context: Option<&super::ParseContext>,
    ) -> Result<Vec<&PureCodec<A, JsonValue>>, SchemaIssue> {
        self.names
            .decode_in_ref(
                &JsonValue::String(name.to_owned()),
                super::parse_context::child(context, IssueSegment::PropertyName(name.into()))?
                    .as_ref(),
            )
            .map_err(|issue| issue.at(IssueSegment::PropertyName(name.into())))?;
        self.routes(name, context)
    }
    /// Select routes after independent borrowed name validation; avoids decoder replay in typed directions.
    fn select_typed(
        &self,
        name: &String,
        context: Option<&super::ParseContext>,
    ) -> Result<Vec<&PureCodec<A, JsonValue>>, SchemaIssue> {
        let child =
            super::parse_context::child(context, IssueSegment::PropertyName(name.as_str().into()))?;
        self.names
            .validate_in_ref(name, child.as_ref())
            .map_err(|issue| issue.at(IssueSegment::PropertyName(name.as_str().into())))?;
        self.routes(name, context)
    }
    fn routes(
        &self,
        name: &str,
        context: Option<&super::ParseContext>,
    ) -> Result<Vec<&PureCodec<A, JsonValue>>, SchemaIssue> {
        let mut codecs = Vec::new();
        if let Some(codec) = self.declared.get(name) {
            codecs.push(codec);
        }
        let mut matched_patterns = 0;
        for pattern in &self.patterns {
            if pattern.pattern.matches_in(name, context)? {
                matched_patterns += 1;
                codecs.push(&pattern.codec);
            }
        }
        if matched_patterns > 1 && matches!(self.multiple_patterns, MultiplePatternPolicy::Reject) {
            return Err(constraint("multiplePatterns", "reject"));
        }
        if codecs.is_empty() {
            if let Some(codec) = &self.additional {
                codecs.push(codec);
            } else {
                return Err(SchemaIssue::new(SchemaIssueKind::UnknownField(name.into())));
            }
        }
        Ok(codecs)
    }
}
/// Build a bounded string-keyed object codec with explicit routing and name validation.
///
/// Rejects reversed or over-policy width bounds. All applicable value routes are
/// validated; encoding merges their outputs and reports conflicts. Property-name
/// failures use PropertyName paths, while value failures use Field paths. Typed
/// validation and direct writing require genuine name/child witnesses and never
/// replay a name decoder to validate an already typed key.
pub fn json_object_constraints<A: Send + 'static>(
    constraints: ObjectConstraints<A>,
    policy: ParsePolicy,
) -> Result<PureCodec<BTreeMap<String, A>, JsonValue>, SchemaIssue> {
    if constraints.minimum > constraints.maximum
        || constraints.maximum > policy.limits.collection.get()
    {
        return Err(invalid());
    }
    let constraints = Arc::new(constraints);
    let decode = Arc::clone(&constraints);
    let validate = Arc::clone(&constraints);
    let write = Arc::clone(&constraints);
    let has_validator = constraints.names.has_type_validator()
        && constraints
            .declared
            .values()
            .all(PureCodec::has_type_validator)
        && constraints
            .patterns
            .iter()
            .all(|p| p.codec.has_type_validator())
        && constraints
            .additional
            .as_ref()
            .is_none_or(PureCodec::has_type_validator);
    let has_writer = has_validator
        && constraints.declared.values().all(PureCodec::has_writer)
        && constraints.patterns.iter().all(|p| p.codec.has_writer())
        && constraints
            .additional
            .as_ref()
            .is_none_or(PureCodec::has_writer);
    let result = PureCodec::with_context_ast(
        SchemaAst::opaque(),
        move |value, context| {
            let policy = super::parse_context::policy(context, policy);
            super::internal::parser::check_limits(value, &policy.limits)?;
            let JsonValue::Object(fields) = value else {
                return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                    expected: ValueKind::Record,
                    actual: value_kind(value),
                }));
            };
            if fields.len() < decode.minimum || fields.len() > decode.maximum {
                return Err(constraint(
                    "propertyCount",
                    format!("{}..={}", decode.minimum, decode.maximum),
                ));
            }
            let mut values = BTreeMap::new();
            for (name, value) in fields {
                let mut selected = None;
                for codec in decode.select(name, context)? {
                    let decoded = codec
                        .decode_in_ref(
                            value,
                            super::parse_context::child(
                                context,
                                IssueSegment::Field(name.as_str().into()),
                            )?
                            .as_ref(),
                        )
                        .map_err(|issue| issue.at(IssueSegment::Field(name.as_str().into())))?;
                    if selected.is_none() {
                        selected = Some(decoded);
                    }
                }
                values.insert(name.clone(), selected.ok_or_else(invalid)?);
            }
            Ok(values)
        },
        move |values: &BTreeMap<String, A>, context| {
            let policy = super::parse_context::policy(context, policy);
            if values.len() < constraints.minimum || values.len() > constraints.maximum {
                return Err(constraint(
                    "propertyCount",
                    format!("{}..={}", constraints.minimum, constraints.maximum),
                ));
            }
            let mut fields = serde_json::Map::new();
            for (name, value) in values {
                let mut encoded = None;
                for codec in constraints.select(name, context)? {
                    let next = codec
                        .encode_in_ref(
                            value,
                            super::parse_context::child(
                                context,
                                IssueSegment::Field(name.as_str().into()),
                            )?
                            .as_ref(),
                        )
                        .map_err(|issue| issue.at(IssueSegment::Field(name.as_str().into())))?;
                    match &mut encoded {
                        Some(previous) => merge(previous, next)?,
                        None => encoded = Some(next),
                    }
                }
                fields.insert(name.clone(), encoded.ok_or_else(invalid)?);
            }
            let encoded = JsonValue::Object(fields);
            super::internal::parser::check_limits(&encoded, &policy.limits)?;
            Ok(encoded)
        },
    );
    let result = if has_validator {
        result.with_context_type_validator(move |values, context| {
            validate_object(
                values,
                &validate,
                super::parse_context::policy(context, policy),
                context,
            )
        })
    } else {
        result
    };
    Ok(if has_writer {
        result.with_writer(move |values, writer| {
            check_object_count(values.len(), &write, policy)?;
            writer.begin_object()?;
            for (name, value) in values {
                let mut encoded = None;
                for codec in write.select_typed(name, None)? {
                    let bytes = codec.write(value, writer.policy())?;
                    let next = super::internal::parser::parse_json_bytes(&bytes, policy)?;
                    match &mut encoded {
                        Some(previous) => merge(previous, next)?,
                        None => encoded = Some(next),
                    }
                }
                writer.property(name)?;
                writer.tree(&encoded.ok_or_else(invalid)?)?;
            }
            writer.end_object()
        })
    } else {
        result
    })
}
fn validate_object<A: Send + 'static>(
    values: &BTreeMap<String, A>,
    constraints: &ObjectConstraints<A>,
    policy: ParsePolicy,
    context: Option<&super::ParseContext>,
) -> Result<(), SchemaIssue> {
    if values.len() > policy.limits.collection.get() {
        return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
            resource: SchemaResource::Width,
        }));
    }
    if values.len() < constraints.minimum || values.len() > constraints.maximum {
        return Err(constraint(
            "propertyCount",
            format!("{}..={}", constraints.minimum, constraints.maximum),
        ));
    }
    for (name, value) in values {
        let name_context =
            super::parse_context::child(context, IssueSegment::PropertyName(name.as_str().into()))?;
        constraints
            .names
            .validate_in_ref(name, name_context.as_ref())
            .map_err(|issue| issue.at(IssueSegment::PropertyName(name.as_str().into())))?;
        let mut codecs = Vec::new();
        if let Some(codec) = constraints.declared.get(name) {
            codecs.push(codec);
        }
        let mut patterns = 0usize;
        for pattern in &constraints.patterns {
            if pattern.pattern.matches_in(name, context)? {
                patterns = patterns.saturating_add(1);
                codecs.push(&pattern.codec);
            }
        }
        if patterns > 1 && matches!(constraints.multiple_patterns, MultiplePatternPolicy::Reject) {
            return Err(constraint("multiplePatterns", "reject"));
        }
        if codecs.is_empty() {
            codecs.push(constraints.additional.as_ref().ok_or_else(|| {
                SchemaIssue::new(SchemaIssueKind::UnknownField(name.as_str().into()))
            })?);
        }
        for codec in codecs {
            let child =
                super::parse_context::child(context, IssueSegment::Field(name.as_str().into()))?;
            codec
                .validate_in_ref(value, child.as_ref())
                .map_err(|issue| issue.at(IssueSegment::Field(name.as_str().into())))?;
        }
    }
    Ok(())
}

fn check_object_count<A>(
    length: usize,
    constraints: &ObjectConstraints<A>,
    policy: ParsePolicy,
) -> Result<(), SchemaIssue> {
    if length > policy.limits.collection.get() {
        return Err(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
            resource: SchemaResource::Width,
        }));
    }
    if length < constraints.minimum || length > constraints.maximum {
        return Err(constraint(
            "propertyCount",
            format!("{}..={}", constraints.minimum, constraints.maximum),
        ));
    }
    Ok(())
}
