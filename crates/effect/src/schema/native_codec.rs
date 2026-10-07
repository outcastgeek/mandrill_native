//! Owned representation adapters and typed effectful record composition.
use super::parse_context::IssueBudget;
use super::{
    Codec, IssueSegment, JsonValue, ParseContext, ParseLimits, ParsePolicy, SchemaAst, SchemaIssue,
    SchemaIssueKind, SchemaProperty, SchemaResource, SchemaShape, UnknownFields, ValueKind,
    value_kind,
};
use crate::Effect;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Ownership-preserving adapter between `Encoded` objects and named encoded fields.
///
/// Implementations must check representation shape and the supplied [`ParseLimits`]
/// without invoking field codecs. Native composites apply duplicate/width checks
/// before application callbacks; an adapter must not silently discard duplicates
/// before those checks can see them. No JSON conversion is required.
pub trait ObjectRepresentation<Encoded>: Send + Sync + 'static {
    /// Consume an object representation and return its owned fields.
    ///
    /// Preserve names and values, and report type or resource violations as
    /// [`SchemaIssue`]. Returning duplicate names allows the composite to reject them.
    fn read(
        &self,
        value: Encoded,
        limits: ParseLimits,
    ) -> Result<Vec<(String, Encoded)>, SchemaIssue>;
    /// Consume encoded fields to build one object under `limits`.
    ///
    /// Reject duplicate names or unsupported representation shape rather than
    /// silently overwriting data; do not run application field callbacks.
    fn write(
        &self,
        fields: Vec<(String, Encoded)>,
        limits: ParseLimits,
    ) -> Result<Encoded, SchemaIssue>;
}
/// Ownership-preserving sequence adapter for an explicit encoded representation.
///
/// Implementations check shape and representation resource bounds; element codec
/// effects belong to the native composite, not the adapter.
pub trait SequenceRepresentation<Encoded>: Send + Sync + 'static {
    /// Consume an encoded sequence and return owned elements in order.
    ///
    /// Check shape and the supplied limits before element callbacks are dispatched;
    /// report representation failures as [`SchemaIssue`].
    fn read(&self, value: Encoded, limits: ParseLimits) -> Result<Vec<Encoded>, SchemaIssue>;
    /// Consume encoded elements in order to build a sequence under `limits`.
    ///
    /// Return representation or resource failures without invoking application codecs.
    fn write(&self, items: Vec<Encoded>, limits: ParseLimits) -> Result<Encoded, SchemaIssue>;
}
/// Object and sequence adapters for [`JsonValue`], with recursive representation limits and duplicate-name rejection.
#[derive(Clone, Copy, Debug)]
pub struct JsonRepresentation;
impl ObjectRepresentation<JsonValue> for JsonRepresentation {
    fn read(
        &self,
        value: JsonValue,
        limits: ParseLimits,
    ) -> Result<Vec<(String, JsonValue)>, SchemaIssue> {
        super::internal::parser::check_limits(&value, &limits)?;
        match value {
            JsonValue::Object(fields) => Ok(fields.into_iter().collect()),
            value @ (JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Array(_)) => Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                expected: ValueKind::Record,
                actual: value_kind(&value),
            })),
        }
    }
    fn write(
        &self,
        fields: Vec<(String, JsonValue)>,
        limits: ParseLimits,
    ) -> Result<JsonValue, SchemaIssue> {
        let mut object = serde_json::Map::new();
        for (key, value) in fields {
            if object.contains_key(&key) {
                return Err(SchemaIssue::new(SchemaIssueKind::DuplicateField(
                    key.into(),
                )));
            }
            object.insert(key, value);
        }
        let value = JsonValue::Object(object);
        super::internal::parser::check_limits(&value, &limits)?;
        Ok(value)
    }
}
impl SequenceRepresentation<JsonValue> for JsonRepresentation {
    fn read(&self, value: JsonValue, limits: ParseLimits) -> Result<Vec<JsonValue>, SchemaIssue> {
        super::internal::parser::check_limits(&value, &limits)?;
        match value {
            JsonValue::Array(items) => Ok(items),
            value @ (JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Object(_)) => Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                expected: ValueKind::Sequence,
                actual: value_kind(&value),
            })),
        }
    }
    fn write(&self, items: Vec<JsonValue>, limits: ParseLimits) -> Result<JsonValue, SchemaIssue> {
        let value = JsonValue::Array(items);
        super::internal::parser::check_limits(&value, &limits)?;
        Ok(value)
    }
}

/// Compose owned elements through a [`SequenceRepresentation`], preserving `D` and `E`.
///
/// The active policy bounds width before element effects. Ordinary schema failures
/// follow first-error or bounded accumulation policy; defects, interruption and
/// cleanup failures keep their outcome channels. Child issues receive index paths.
/// A direct JSON writer is exposed only when the element codec has that witness.
pub fn native_array<A, Encoded, D, E, Repr>(
    codec: Codec<A, Encoded, D, E>,
    representation: Repr,
    policy: ParsePolicy,
) -> Codec<Vec<A>, Encoded, D, E>
where
    A: Send + 'static,
    Encoded: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    Repr: SequenceRepresentation<Encoded>,
{
    let ast = SchemaAst::new(SchemaShape::Sequence {
        element: codec.ast().clone(),
        maximum: Some(policy.limits.collection),
    });
    let decode = codec.clone();
    let validate = codec.clone();
    let writer_codec = codec.clone();
    let writer_available = codec.has_writer();
    let repr = Arc::new(representation);
    let read = Arc::clone(&repr);
    let result = Codec::with_context(
        ast,
        move |input, context| {
            let policy = active_policy::<Encoded>(&context, policy);
            let decode = decode.clone();
            let read = Arc::clone(&read);
            Effect::try_sync_once(
                move || {
                    let items = read.read(input, policy.limits)?;
                    if items.len() > policy.limits.collection.get() {
                        return Err(width());
                    }
                    Ok(items)
                },
                std::convert::identity,
            )
            .flat_map(move |items| {
                all_schema(
                    items
                        .into_iter()
                        .enumerate()
                        .map(|(index, item)| {
                            match child_context(&context, IssueSegment::Index(index)) {
                                Ok(child) => decode.decode_in(item, child),
                                Err(issue) => Effect::from_result_once(Err(issue)),
                            }
                            .map_error(move |issue| issue.at(IssueSegment::Index(index)))
                        })
                        .collect(),
                    policy,
                    budget_for(&context, policy),
                )
            })
        },
        move |items: Vec<A>, context| {
            let policy = active_policy::<Encoded>(&context, policy);
            if items.len() > policy.limits.collection.get() {
                return Effect::from_result_once(Err(width()));
            }
            let repr = Arc::clone(&repr);
            all_schema(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(index, item)| {
                        match child_context(&context, IssueSegment::Index(index)) {
                            Ok(child) => codec.encode_in(item, child),
                            Err(issue) => Effect::from_result_once(Err(issue)),
                        }
                        .map_error(move |issue| issue.at(IssueSegment::Index(index)))
                    })
                    .collect(),
                policy,
                budget_for(&context, policy),
            )
            .flat_map(move |items| {
                let repr = Arc::clone(&repr);
                Effect::try_sync_once(
                    move || repr.write(items, policy.limits),
                    std::convert::identity,
                )
            })
        },
    )
    .with_context_validator(move |items: Vec<A>, context| {
        let policy = active_policy::<Encoded>(&context, policy);
        if items.len() > policy.limits.collection.get() {
            return Effect::from_result_once(Err(width()));
        }
        all_schema(
            items
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    match child_context(&context, IssueSegment::Index(index)) {
                        Ok(child) => validate.validate_in(item, child),
                        Err(issue) => Effect::from_result_once(Err(issue)),
                    }
                    .map_error(move |issue| issue.at(IssueSegment::Index(index)))
                })
                .collect(),
            policy,
            budget_for(&context, policy),
        )
    });
    if writer_available {
        result.with_writer(move |items: Vec<A>, mut writer| {
            if items.len() > policy.limits.collection.get() {
                return Effect::from_result_once(Err(width()));
            }
            if let Err(issue) = writer.begin_array() {
                return Effect::from_result_once(Err(issue));
            }
            let mut result = Effect::from_result_once(Ok(writer));
            for item in items {
                let codec = writer_codec.clone();
                result = result
                    .zip(Effect::from_result_once(Ok(item)))
                    .flat_map(move |(writer, item)| codec.write_into_once(item, writer));
            }
            result.flat_map(|mut writer| {
                Effect::from_result_once(writer.end_array().map(|()| writer))
            })
        })
    } else {
        result
    }
}
fn width() -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::LimitExceeded {
        resource: SchemaResource::Width,
    })
}

/// Compose owned string-keyed values through an [`ObjectRepresentation`].
///
/// Rejects duplicate names and excessive width before element effects. Decode uses
/// `D`; encode and validation use `E`, without cloning payloads. Child failures
/// receive field paths and follow the active error policy. JSON writing requires
/// a genuine writer witness on the element codec.
pub fn native_map<A, Encoded, D, E, Repr>(
    codec: Codec<A, Encoded, D, E>,
    representation: Repr,
    policy: ParsePolicy,
) -> Codec<BTreeMap<String, A>, Encoded, D, E>
where
    A: Send + 'static,
    Encoded: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    Repr: ObjectRepresentation<Encoded>,
{
    let decode = codec.clone();
    let validate = codec.clone();
    let writer_codec = codec.clone();
    let writer_available = codec.has_writer();
    let repr = Arc::new(representation);
    let read = Arc::clone(&repr);
    let result = Codec::with_context(
        SchemaAst::opaque(),
        move |input, context| {
            let policy = active_policy::<Encoded>(&context, policy);
            let read = Arc::clone(&read);
            let decode = decode.clone();
            Effect::try_sync_once(
                move || checked_fields(read.read(input, policy.limits)?, policy.limits),
                std::convert::identity,
            )
            .flat_map(move |fields| {
                all_schema(
                    fields
                        .into_iter()
                        .map(|(name, item)| {
                            let path: Arc<str> = Arc::from(name.as_str());
                            match child_context(&context, IssueSegment::Field(Arc::clone(&path))) {
                                Ok(child) => decode.decode_in(item, child),
                                Err(issue) => Effect::from_result_once(Err(issue)),
                            }
                            .map_error(move |issue| {
                                issue.at(IssueSegment::Field(Arc::clone(&path)))
                            })
                            .zip(Effect::from_result_once(Ok(name)))
                            .map(|(value, name)| (name, value))
                        })
                        .collect(),
                    policy,
                    budget_for(&context, policy),
                )
                .map(|values| values.into_iter().collect())
            })
        },
        move |items: BTreeMap<String, A>, context| {
            let policy = active_policy::<Encoded>(&context, policy);
            if items.len() > policy.limits.collection.get() {
                return Effect::from_result_once(Err(width()));
            }
            let repr = Arc::clone(&repr);
            all_schema(
                items
                    .into_iter()
                    .map(|(name, item)| {
                        let path: Arc<str> = Arc::from(name.as_str());
                        match child_context(&context, IssueSegment::Field(Arc::clone(&path))) {
                            Ok(child) => codec.encode_in(item, child),
                            Err(issue) => Effect::from_result_once(Err(issue)),
                        }
                        .map_error(move |issue| issue.at(IssueSegment::Field(Arc::clone(&path))))
                        .zip(Effect::from_result_once(Ok(name)))
                        .map(|(value, name)| (name, value))
                    })
                    .collect(),
                policy,
                budget_for(&context, policy),
            )
            .flat_map(move |fields| {
                let repr = Arc::clone(&repr);
                Effect::try_sync_once(
                    move || repr.write(fields, policy.limits),
                    std::convert::identity,
                )
            })
        },
    )
    .with_context_validator(move |items: BTreeMap<String, A>, context| {
        let policy = active_policy::<Encoded>(&context, policy);
        if items.len() > policy.limits.collection.get() {
            return Effect::from_result_once(Err(width()));
        }
        all_schema(
            items
                .into_iter()
                .map(|(name, item)| {
                    let path: Arc<str> = Arc::from(name.as_str());
                    match child_context(&context, IssueSegment::Field(Arc::clone(&path))) {
                        Ok(child) => validate.validate_in(item, child),
                        Err(issue) => Effect::from_result_once(Err(issue)),
                    }
                    .map_error(move |issue| issue.at(IssueSegment::Field(Arc::clone(&path))))
                    .zip(Effect::from_result_once(Ok(name)))
                    .map(|(value, name)| (name, value))
                })
                .collect(),
            policy,
            budget_for(&context, policy),
        )
        .map(|values| values.into_iter().collect())
    });
    if writer_available {
        result.with_writer(move |items: BTreeMap<String, A>, mut writer| {
            if items.len() > policy.limits.collection.get() {
                return Effect::from_result_once(Err(width()));
            }
            if let Err(issue) = writer.begin_object() {
                return Effect::from_result_once(Err(issue));
            }
            let mut result = Effect::from_result_once(Ok(writer));
            for (name, item) in items {
                let codec = writer_codec.clone();
                result = result
                    .zip(Effect::from_result_once(Ok((name, item))))
                    .flat_map(move |(mut writer, (name, item))| {
                        if let Err(issue) = writer.property(&name) {
                            return Effect::from_result_once(Err(issue));
                        }
                        codec.write_into_once(item, writer)
                    });
            }
            result.flat_map(|mut writer| {
                Effect::from_result_once(writer.end_object().map(|()| writer))
            })
        })
    } else {
        result
    }
}
fn checked_fields<Encoded>(
    fields: Vec<(String, Encoded)>,
    limits: ParseLimits,
) -> Result<BTreeMap<String, Encoded>, SchemaIssue> {
    if fields.len() > limits.collection.get() {
        return Err(width());
    }
    let mut object = BTreeMap::new();
    for (name, value) in fields {
        if object.contains_key(&name) {
            return Err(SchemaIssue::new(SchemaIssueKind::DuplicateField(
                name.into(),
            )));
        }
        object.insert(name, value);
    }
    Ok(object)
}

/// The empty tail of a typed native field list; its owned value shape is `()`.
pub struct NativeNil;
/// A field codec and its absence/omission rules linked to the remaining typed fields.
pub struct NativeField<
    F: Send + 'static,
    Encoded: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    Rest,
> {
    name: Arc<str>,
    codec: Codec<F, Encoded, D, E>,
    missing: Option<DefaultFactory<F, D>>,
    default_validator: Option<DefaultValidator<F>>,
    omit: Option<OmitField<F>>,
    rest: Arc<Rest>,
}
/// Typed field-list contract used by [`NativeRecordBuilder`] and generated native records.
///
/// Custom implementations must preserve the active context, policy and shared
/// issue budget in nested work. Add relative field paths once, return owned values,
/// and accumulate ordinary schema issues only within policy bounds. Defects,
/// interruption and cleanup failures must retain their original outcome channels.
pub trait NativeFields<Encoded: Send + 'static, D: Send + Sync + 'static, E: Send + Sync + 'static>:
    Send + Sync + 'static
{
    /// Owned heterogeneous field values; each built-in field adds `(Field, Rest)` to the front.
    type Values: Send + 'static;
    /// Whether every field has a genuine direct JSON writer; metadata alone is insufficient.
    fn writer_available(&self) -> bool;
    /// Lazily append each non-omitted field to an already-open JSON object in `E`.
    ///
    /// The caller owns the surrounding object and its policy. Return the same writer;
    /// do not close or replace it, and use field codecs' checked writer witnesses.
    fn write(
        &self,
        values: Self::Values,
        writer: super::JsonWriter,
    ) -> Effect<super::JsonWriter, SchemaIssue, E>;
    /// Append field metadata, returning a schema issue for invalid names or definitions.
    fn properties(&self, output: &mut Vec<SchemaProperty>) -> Result<(), SchemaIssue>;
    /// Remove this list's owned fields from `object` and lazily decode them in `D`.
    ///
    /// Only absence selects missing/default factories. Pass `context`, `policy` and
    /// `budget` through nested field work; present null is an ordinary encoded value.
    fn decode(
        &self,
        object: &mut BTreeMap<String, Encoded>,
        context: Option<ParseContext>,
        policy: ParsePolicy,
        budget: IssueBudget,
    ) -> Effect<Self::Values, SchemaIssue, D>;
    /// Lazily consume typed fields in `E` and return encoded name/value pairs.
    ///
    /// Apply omission rules and the active bounded error policy without cloning values
    /// or replacing shared invocation accounting.
    fn encode(
        &self,
        values: Self::Values,
        context: Option<ParseContext>,
        policy: ParsePolicy,
        budget: IssueBudget,
    ) -> Effect<Vec<(String, Encoded)>, SchemaIssue, E>;
    /// Lazily check owned field values in `E`, returning them on success.
    ///
    /// Use independent field validation where supplied and retain the active context,
    /// error policy and issue accounting. Do not encode defaults to validate them.
    fn validate(
        &self,
        values: Self::Values,
        context: Option<ParseContext>,
        policy: ParsePolicy,
        budget: IssueBudget,
    ) -> Effect<Self::Values, SchemaIssue, E>;
}
impl<Encoded: Send + 'static, D: Send + Sync + 'static, E: Send + Sync + 'static>
    NativeFields<Encoded, D, E> for NativeNil
{
    type Values = ();
    fn writer_available(&self) -> bool {
        true
    }
    fn write(&self, _: (), writer: super::JsonWriter) -> Effect<super::JsonWriter, SchemaIssue, E> {
        Effect::from_result_once(Ok(writer))
    }
    fn properties(&self, _: &mut Vec<SchemaProperty>) -> Result<(), SchemaIssue> {
        Ok(())
    }
    fn decode(
        &self,
        _: &mut BTreeMap<String, Encoded>,
        _: Option<ParseContext>,
        _: ParsePolicy,
        _: IssueBudget,
    ) -> Effect<(), SchemaIssue, D> {
        Effect::succeed(())
    }
    fn encode(
        &self,
        _: (),
        _: Option<ParseContext>,
        _: ParsePolicy,
        _: IssueBudget,
    ) -> Effect<Vec<(String, Encoded)>, SchemaIssue, E> {
        Effect::sync(Vec::new)
    }
    fn validate(
        &self,
        _: (),
        _: Option<ParseContext>,
        _: ParsePolicy,
        _: IssueBudget,
    ) -> Effect<(), SchemaIssue, E> {
        Effect::succeed(())
    }
}
impl<F, Encoded, D, E, Rest> NativeFields<Encoded, D, E> for NativeField<F, Encoded, D, E, Rest>
where
    F: Send + 'static,
    Encoded: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    Rest: NativeFields<Encoded, D, E>,
{
    type Values = (F, Rest::Values);
    fn writer_available(&self) -> bool {
        self.codec.has_writer() && self.rest.writer_available()
    }
    fn write(
        &self,
        (value, rest): Self::Values,
        mut writer: super::JsonWriter,
    ) -> Effect<super::JsonWriter, SchemaIssue, E> {
        let field = if self.omit.as_ref().is_some_and(|omit| omit(&value)) {
            Effect::from_result_once(Ok(writer))
        } else {
            if let Err(issue) = writer.property(&self.name) {
                return Effect::from_result_once(Err(issue));
            }
            self.codec.write_into_once(value, writer)
        };
        let fields = Arc::clone(&self.rest);
        field
            .zip(Effect::from_result_once(Ok(rest)))
            .flat_map(move |(writer, rest)| fields.write(rest, writer))
    }

    fn properties(&self, output: &mut Vec<SchemaProperty>) -> Result<(), SchemaIssue> {
        output.push(SchemaProperty::new(
            Arc::clone(&self.name),
            self.codec.ast().clone(),
            self.missing.is_none(),
            None,
        )?);
        self.rest.properties(output)
    }
    fn decode(
        &self,
        object: &mut BTreeMap<String, Encoded>,
        context: Option<ParseContext>,
        policy: ParsePolicy,
        budget: IssueBudget,
    ) -> Effect<Self::Values, SchemaIssue, D> {
        let field = match object.remove(self.name.as_ref()) {
            Some(value) => {
                match child_context(&context, IssueSegment::Field(Arc::clone(&self.name))) {
                    Ok(child) => self.codec.decode_in(value, child),
                    Err(issue) => Effect::from_result_once(Err(issue)),
                }
            }
            None => match &self.missing {
                Some(default) => {
                    let default = Arc::clone(default);
                    let validate = self.default_validator.as_ref().map(Arc::clone);
                    let child = match child_context(
                        &context,
                        IssueSegment::Field(Arc::clone(&self.name)),
                    ) {
                        Ok(child) => child,
                        Err(issue) => {
                            return Effect::from_result_once(Err(
                                issue.at(IssueSegment::Field(Arc::clone(&self.name)))
                            ));
                        }
                    };
                    Effect::defer(move || default()).flat_map(move |value| {
                        let validate = validate.as_ref().map(Arc::clone);
                        let child = child.clone();
                        Effect::try_sync_once(
                            move || {
                                if let Some(validate) = validate {
                                    validate(&value, child.as_ref())?;
                                }
                                Ok(value)
                            },
                            std::convert::identity,
                        )
                    })
                }
                None => {
                    let name = Arc::clone(&self.name);
                    Effect::fail_with(move || {
                        SchemaIssue::new(SchemaIssueKind::MissingField(Arc::clone(&name)))
                    })
                }
            },
        };
        let name = Arc::clone(&self.name);
        let field = field.map_error(move |issue| issue.at(IssueSegment::Field(Arc::clone(&name))));
        let rest = self
            .rest
            .decode(object, context.clone(), policy, budget.clone());
        collect_pair(field, rest, policy, budget)
    }
    fn encode(
        &self,
        (value, rest): Self::Values,
        context: Option<ParseContext>,
        policy: ParsePolicy,
        budget: IssueBudget,
    ) -> Effect<Vec<(String, Encoded)>, SchemaIssue, E> {
        let field = if self.omit.as_ref().is_some_and(|omit| omit(&value)) {
            Effect::sync(Vec::new)
        } else {
            let path = Arc::clone(&self.name);
            let child = match child_context(&context, IssueSegment::Field(Arc::clone(&self.name))) {
                Ok(child) => child,
                Err(issue) => {
                    return Effect::from_result_once(Err(
                        issue.at(IssueSegment::Field(Arc::clone(&self.name)))
                    ));
                }
            };
            self.codec
                .encode_in(value, child)
                .map_error(move |issue| issue.at(IssueSegment::Field(Arc::clone(&path))))
                .zip(Effect::from_result_once(Ok(self.name.to_string())))
                .map(|(value, name)| vec![(name, value)])
        };
        collect_pair(
            field,
            self.rest.encode(rest, context, policy, budget.clone()),
            policy,
            budget,
        )
        .map(|(mut field, rest)| {
            field.extend(rest);
            field
        })
    }
    fn validate(
        &self,
        (value, rest): Self::Values,
        context: Option<ParseContext>,
        policy: ParsePolicy,
        budget: IssueBudget,
    ) -> Effect<Self::Values, SchemaIssue, E> {
        let field = if self.omit.as_ref().is_some_and(|omit| omit(&value)) {
            Effect::from_result_once(Ok(value))
        } else {
            match child_context(&context, IssueSegment::Field(Arc::clone(&self.name))) {
                Ok(child) => self.codec.validate_in(value, child),
                Err(issue) => {
                    return Effect::from_result_once(Err(
                        issue.at(IssueSegment::Field(Arc::clone(&self.name)))
                    ));
                }
            }
        };
        let name = Arc::clone(&self.name);
        let field = field.map_error(move |issue| issue.at(IssueSegment::Field(Arc::clone(&name))));
        collect_pair(
            field,
            self.rest.validate(rest, context, policy, budget.clone()),
            policy,
            budget,
        )
    }
}

type DefaultFactory<A, D> = Arc<dyn Fn() -> Effect<A, SchemaIssue, D> + Send + Sync>;
type DefaultValidator<A> =
    Arc<dyn Fn(&A, Option<&ParseContext>) -> Result<(), SchemaIssue> + Send + Sync>;
type OmitField<A> = Arc<dyn Fn(&A) -> bool + Send + Sync>;
type RecordMarker<Encoded, D, E> = std::marker::PhantomData<fn() -> (Encoded, D, E)>;
/// A typed, ownership-preserving record builder with separate decode and encode environments.
///
/// Each added field becomes the head of a nested tuple; the most recently added
/// field is destructured first. [`Self::finish`] supplies explicit representation,
/// unknown-field policy and constructor/destructor callbacks. The `schema!` macro
/// builds the same machinery for named Rust fields.
pub struct NativeRecordBuilder<Encoded, D, E, Fields = NativeNil> {
    fields: Fields,
    marker: RecordMarker<Encoded, D, E>,
}
impl<Encoded, D, E> Default for NativeRecordBuilder<Encoded, D, E> {
    fn default() -> Self {
        Self::new()
    }
}
impl<Encoded, D, E> NativeRecordBuilder<Encoded, D, E> {
    /// Start an empty typed field list with no inferred representation or policy.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            fields: NativeNil,
            marker: std::marker::PhantomData,
        }
    }
}
impl<Encoded: Send + 'static, D: Send + Sync + 'static, E: Send + Sync + 'static, Fields>
    NativeRecordBuilder<Encoded, D, E, Fields>
{
    /// Add a required field. Decode reports `MissingField` only when its name is absent.
    ///
    /// Present null goes to `codec`; the constructor receives the owned decoded value.
    #[must_use]
    pub fn field<F: Send + 'static>(
        self,
        name: impl Into<Arc<str>>,
        codec: Codec<F, Encoded, D, E>,
    ) -> NativeRecordBuilder<Encoded, D, E, NativeField<F, Encoded, D, E, Fields>> {
        NativeRecordBuilder {
            fields: NativeField {
                name: name.into(),
                codec,
                missing: None,
                default_validator: None,
                omit: None,
                rest: Arc::new(self.fields),
            },
            marker: std::marker::PhantomData,
        }
    }
    /// Add an absent-field default factory in `D` and an independent borrowed validator.
    ///
    /// The factory runs lazily only for absence, once for that field's decode. Its owned
    /// value is validated and passed directly to the record constructor; it is never
    /// encoded and decoded again. Present null does not select the default. Use
    /// [`Self::default_with_context`] to preserve configured invocation accounting.
    #[must_use]
    pub fn default_with<F: Send + 'static>(
        self,
        name: impl Into<Arc<str>>,
        codec: Codec<F, Encoded, D, E>,
        default: impl Fn() -> Effect<F, SchemaIssue, D> + Send + Sync + 'static,
        validate: impl Fn(&F) -> Result<(), SchemaIssue> + Send + Sync + 'static,
    ) -> NativeRecordBuilder<Encoded, D, E, NativeField<F, Encoded, D, E, Fields>> {
        NativeRecordBuilder {
            fields: NativeField {
                name: name.into(),
                codec,
                missing: Some(Arc::new(default)),
                default_validator: Some(Arc::new(move |value, _| validate(value))),
                omit: None,
                rest: Arc::new(self.fields),
            },
            marker: std::marker::PhantomData,
        }
    }
    /// Add an absent-field default with a context-aware independent validator.
    ///
    /// `validate` receives the decoded child's invocation context, including shared
    /// budgets. It must check the borrowed default directly, without using encode
    /// capabilities or cloning the payload. Factory and validation errors receive the
    /// field's path; present null still goes through the ordinary decoder.
    #[must_use]
    pub fn default_with_context<F: Send + 'static>(
        self,
        name: impl Into<Arc<str>>,
        codec: Codec<F, Encoded, D, E>,
        default: impl Fn() -> Effect<F, SchemaIssue, D> + Send + Sync + 'static,
        validate: impl Fn(&F, Option<&ParseContext>) -> Result<(), SchemaIssue> + Send + Sync + 'static,
    ) -> NativeRecordBuilder<Encoded, D, E, NativeField<F, Encoded, D, E, Fields>> {
        NativeRecordBuilder {
            fields: NativeField {
                name: name.into(),
                codec,
                missing: Some(Arc::new(default)),
                default_validator: Some(Arc::new(validate)),
                omit: None,
                rest: Arc::new(self.fields),
            },
            marker: std::marker::PhantomData,
        }
    }
    /// Add explicit missing-value and encode-omission rules.
    ///
    /// `missing` creates an owned value only when the field is absent. `omit` decides
    /// whether encoding, validation and direct writing skip the field. Unlike a
    /// validated default, the absence factory has no separate borrowed validator;
    /// callers must supply a valid absent value.
    #[must_use]
    pub fn optional<F: Send + 'static>(
        self,
        name: impl Into<Arc<str>>,
        codec: Codec<F, Encoded, D, E>,
        missing: impl Fn() -> F + Send + Sync + 'static,
        omit: impl Fn(&F) -> bool + Send + Sync + 'static,
    ) -> NativeRecordBuilder<Encoded, D, E, NativeField<F, Encoded, D, E, Fields>> {
        NativeRecordBuilder {
            fields: NativeField {
                name: name.into(),
                codec,
                missing: Some({
                    let missing = Arc::new(missing);
                    Arc::new(move || {
                        let missing = Arc::clone(&missing);
                        Effect::sync(move || missing())
                    })
                }),
                default_validator: None,
                omit: Some(Arc::new(omit)),
                rest: Arc::new(self.fields),
            },
            marker: std::marker::PhantomData,
        }
    }
}
impl<
    Encoded: Send + 'static,
    D: Send + Sync + 'static,
    E: Send + Sync + 'static,
    Fields: NativeFields<Encoded, D, E>,
> NativeRecordBuilder<Encoded, D, E, Fields>
{
    /// Validate field metadata and prepare a record codec under the explicit policies.
    ///
    /// `construct` consumes the nested field tuple after successful decoding or
    /// validation; `destruct` consumes `T` for encoding and validation. These callbacks
    /// must agree with the field order. Invalid names, duplicate definitions or
    /// preparation limits fail eagerly; field effects remain lazy. Unknown-field
    /// handling applies before construction, and writers are available only when all
    /// fields have genuine witnesses.
    pub fn finish<T: Send + 'static, Repr: ObjectRepresentation<Encoded>>(
        self,
        representation: Repr,
        policy: ParsePolicy,
        unknown_fields: UnknownFields,
        construct: impl Fn(Fields::Values) -> T + Send + Sync + 'static,
        destruct: impl Fn(T) -> Fields::Values + Send + Sync + 'static,
    ) -> Result<Codec<T, Encoded, D, E>, SchemaIssue> {
        let mut properties = Vec::new();
        self.fields.properties(&mut properties)?;
        let known: BTreeSet<_> = properties
            .iter()
            .map(|p| Arc::<str>::from(p.name()))
            .collect();
        let ast = SchemaAst::object(properties, unknown_fields)?;
        ast.prepare(policy.limits)?;
        let fields = Arc::new(self.fields);
        let writer_fields = Arc::clone(&fields);
        let writer_available = fields.writer_available();
        let decode = Arc::clone(&fields);
        let validate = Arc::clone(&fields);
        let repr = Arc::new(representation);
        let read = Arc::clone(&repr);
        let construct = Arc::new(construct);
        let constructor = Arc::clone(&construct);
        let destruct = Arc::new(destruct);
        let destructor = Arc::clone(&destruct);
        let writer_destruct = Arc::clone(&destruct);
        let codec = Codec::with_context(
            ast,
            move |input, context| {
                let policy = active_policy::<Encoded>(&context, policy);
                let read = Arc::clone(&read);
                let known = known.clone();
                let decode = Arc::clone(&decode);
                let construct = Arc::clone(&constructor);
                Effect::try_sync_once(
                    move || {
                        let object =
                            checked_fields(read.read(input, policy.limits)?, policy.limits)?;
                        if unknown_fields == UnknownFields::Reject
                            && let Some(name) =
                                object.keys().find(|name| !known.contains(name.as_str()))
                        {
                            return Err(SchemaIssue::new(SchemaIssueKind::UnknownField(
                                Arc::from(name.as_str()),
                            )));
                        }
                        Ok(object)
                    },
                    std::convert::identity,
                )
                .flat_map(move |mut object| {
                    decode
                        .decode(
                            &mut object,
                            context.clone(),
                            policy,
                            budget_for(&context, policy),
                        )
                        .map({
                            let construct = Arc::clone(&construct);
                            move |values| construct(values)
                        })
                })
            },
            move |value, context| {
                let policy = active_policy::<Encoded>(&context, policy);
                let repr = Arc::clone(&repr);
                fields
                    .encode(
                        destructor(value),
                        context.clone(),
                        policy,
                        budget_for(&context, policy),
                    )
                    .flat_map(move |fields| {
                        let repr = Arc::clone(&repr);
                        Effect::try_sync_once(
                            move || repr.write(fields, policy.limits),
                            std::convert::identity,
                        )
                    })
            },
        )
        .with_context_validator(move |value, context| {
            let policy = active_policy::<Encoded>(&context, policy);
            validate
                .validate(
                    destruct(value),
                    context.clone(),
                    policy,
                    budget_for(&context, policy),
                )
                .map({
                    let construct = Arc::clone(&construct);
                    move |values| construct(values)
                })
        });
        Ok(if writer_available {
            codec.with_writer(move |value, mut writer| {
                if let Err(issue) = writer.begin_object() {
                    return Effect::from_result_once(Err(issue));
                }
                writer_fields
                    .write(writer_destruct(value), writer)
                    .flat_map(|mut writer| {
                        Effect::from_result_once(writer.end_object().map(|()| writer))
                    })
            })
        } else {
            codec
        })
    }
}

fn child_context(
    context: &Option<ParseContext>,
    segment: IssueSegment,
) -> Result<Option<ParseContext>, SchemaIssue> {
    context
        .as_ref()
        .map(|context| context.child(segment))
        .transpose()
}

fn resource_issue(issue: &SchemaIssue) -> bool {
    matches!(
        issue.kind(),
        SchemaIssueKind::LimitExceeded { .. }
            | SchemaIssueKind::ResourceDemand { .. }
            | SchemaIssueKind::PatternBudget
    ) || issue.children().iter().any(resource_issue)
}
fn active_policy<Encoded: 'static>(
    context: &Option<ParseContext>,
    fallback: ParsePolicy,
) -> ParsePolicy {
    context.as_ref().map_or(fallback, |context| {
        let mut policy = context.policy().parse;
        if std::any::TypeId::of::<Encoded>() == std::any::TypeId::of::<JsonValue>() {
            policy.limits.depth = context.policy().json_depth;
        }
        policy
    })
}
fn budget_for(context: &Option<ParseContext>, policy: ParsePolicy) -> IssueBudget {
    context.as_ref().map_or_else(
        || IssueBudget::new(policy.limits.issues),
        ParseContext::issue_budget,
    )
}
/// Evaluate field and tail with bounded schema-issue accumulation, preserving
/// non-schema outcomes and charging composite wrappers without double-counting children.
fn collect_pair<A: Send + 'static, B: Send + 'static, R: Send + Sync + 'static>(
    first: Effect<A, SchemaIssue, R>,
    second: Effect<B, SchemaIssue, R>,
    policy: ParsePolicy,
    budget: IssueBudget,
) -> Effect<(A, B), SchemaIssue, R> {
    if policy.errors == super::SchemaErrorMode::First {
        return first.zip(second);
    }
    Effect::try_sync(
        {
            let budget = budget.clone();
            move || {
                budget.available()?;
                Ok(budget.used())
            }
        },
        std::convert::identity,
    )
    .flat_map(move |before| {
        let second = second.clone();
        let budget = budget.clone();
        first.clone().either().flat_map(move |first| {
            if let Err(issue) = &first {
                if resource_issue(issue) {
                    return Effect::from_result_once(Err(issue.clone()));
                }
                if let Err(issue) = budget
                    .record(before, issue)
                    .and_then(|()| budget.available())
                {
                    return Effect::from_result_once(Err(issue));
                }
            }
            let budget = budget.clone();
            let before_second = budget.used();
            second
                .clone()
                .either()
                .zip(Effect::from_result_once(Ok(first)))
                .flat_map(move |(second, first)| {
                    if let Err(issue) = &second {
                        if resource_issue(issue) {
                            return Effect::from_result_once(Err(issue.clone()));
                        }
                        if let Err(issue) = budget.record(before_second, issue) {
                            return Effect::from_result_once(Err(issue));
                        }
                    }
                    let outcome = match (first, second) {
                        (Ok(first), Ok(second)) => Ok((first, second)),
                        (Err(issue), Ok(_)) | (Ok(_), Err(issue)) => Err(issue),
                        (Err(first), Err(second)) => {
                            Err(SchemaIssue::new(SchemaIssueKind::Multiple)
                                .with_children(vec![first, second]))
                        }
                    };
                    if let Err(issue) = &outcome
                        && let Err(limit) = budget.record(before, issue)
                    {
                        return Effect::from_result_once(Err(limit));
                    }
                    Effect::from_result_once(outcome)
                })
        })
    })
}
fn all_schema<A: Send + 'static, R: Send + Sync + 'static>(
    effects: Vec<Effect<A, SchemaIssue, R>>,
    policy: ParsePolicy,
    budget: IssueBudget,
) -> Effect<Vec<A>, SchemaIssue, R> {
    if policy.errors == super::SchemaErrorMode::First {
        return Effect::all(effects);
    }
    let mut collected = Effect::from_result_once(Ok((Vec::new(), Vec::<SchemaIssue>::new())));
    let beginning = budget.used();
    for effect in effects {
        let budget = budget.clone();
        collected = collected.flat_map(move |state| {
            if let Err(issue) = budget.available() {
                return Effect::from_result_once(Err(issue));
            }
            let before = budget.used();
            let budget = budget.clone();
            effect
                .clone()
                .either()
                .zip(Effect::from_result_once(Ok(state)))
                .flat_map(move |(result, (mut values, mut issues))| {
                    match result {
                        Ok(value) => values.push(value),
                        Err(issue) => {
                            if resource_issue(&issue) {
                                return Effect::from_result_once(Err(issue));
                            }
                            if let Err(limit) = budget.record(before, &issue) {
                                return Effect::from_result_once(Err(limit));
                            }
                            issues.push(issue);
                        }
                    }
                    Effect::from_result_once(Ok((values, issues)))
                })
        });
    }
    collected.flat_map(move |(values, mut issues)| {
        let result = if issues.is_empty() {
            Ok(values)
        } else if issues.len() == 1 {
            Err(issues.remove(0))
        } else {
            Err(SchemaIssue::new(SchemaIssueKind::Multiple).with_children(issues))
        };
        if let Err(issue) = &result
            && let Err(limit) = budget.record(beginning, issue)
        {
            return Effect::from_result_once(Err(limit));
        }
        Effect::from_result_once(result)
    })
}
