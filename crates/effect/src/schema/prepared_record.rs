//! Managed representation backend for pure record codecs.
use super::{
    JsonValue, JsonWriter, ParseContext, ParsePolicy, PreparedIssueAccumulator,
    PreparedJsonArrayPool, PreparedJsonKind, PreparedJsonParserPool, PreparedJsonValidationPool,
    PureCodec, SchemaAst, SchemaIssue, SchemaIssueKind, SchemaProperty, SharedPreparedJsonValue,
    UnknownFields, ValueKind,
};
use std::{collections::BTreeSet, marker::PhantomData, sync::Arc};

/// Existing child-array pool or a shared diagnostic service for record accumulation.
pub enum PreparedRecordIssues {
    /// Independently supplied array pool.
    Array(Arc<PreparedJsonArrayPool<SchemaIssue>>),
    /// Child arrays shared with prepared diagnostics.
    Memory(Arc<super::PreparedIssueMemory>),
}
impl From<Arc<PreparedJsonArrayPool<SchemaIssue>>> for PreparedRecordIssues {
    fn from(value: Arc<PreparedJsonArrayPool<SchemaIssue>>) -> Self {
        Self::Array(value)
    }
}
impl From<Arc<super::PreparedIssueMemory>> for PreparedRecordIssues {
    fn from(value: Arc<super::PreparedIssueMemory>) -> Self {
        Self::Memory(value)
    }
}
impl PreparedRecordIssues {
    fn pool(&self) -> &PreparedJsonArrayPool<SchemaIssue> {
        match self {
            Self::Array(pool) => pool,
            Self::Memory(memory) => memory.child_pool(),
        }
    }
    fn issue_memory(&self) -> Option<&super::PreparedIssueMemory> {
        match self {
            Self::Array(_) => None,
            Self::Memory(memory) => Some(memory),
        }
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::PreparedRecordNil {}
    impl<T, F, R: Sealed> Sealed for super::PreparedRecordField<T, F, R> {}
}
/// Sealed typed managed-record field list.
pub trait PreparedRecordFields<T>: sealed::Sealed + Send + Sync + 'static {
    /// Values consumed by the record constructor.
    type Values: Send + 'static;
    #[doc(hidden)]
    fn decode(
        &self,
        input: &SharedPreparedJsonValue,
        issues: &mut PreparedIssueAccumulator<'_>,
        context: Option<&ParseContext>,
    ) -> Result<Option<Self::Values>, SchemaIssue>;
    #[doc(hidden)]
    fn encode(
        &self,
        value: &T,
        output: &mut super::internal::parser::PreparedObjectAssembly,
        context: Option<&ParseContext>,
    ) -> Result<(), SchemaIssue>;
    #[doc(hidden)]
    fn properties(&self, result: &mut Vec<SchemaProperty>) -> Result<(), SchemaIssue>;
    #[doc(hidden)]
    fn validator_available(&self) -> bool;
    #[doc(hidden)]
    fn validate(&self, value: &T, context: Option<&ParseContext>) -> Result<(), SchemaIssue>;
    #[doc(hidden)]
    fn writer_available(&self) -> bool;
    #[doc(hidden)]
    fn write(&self, value: &T, writer: &mut JsonWriter) -> Result<(), SchemaIssue>;
}
/// Empty field list.
pub struct PreparedRecordNil;
impl<T> PreparedRecordFields<T> for PreparedRecordNil {
    type Values = ();
    fn decode(
        &self,
        _: &SharedPreparedJsonValue,
        _: &mut PreparedIssueAccumulator<'_>,
        _: Option<&ParseContext>,
    ) -> Result<Option<()>, SchemaIssue> {
        Ok(Some(()))
    }
    fn encode(
        &self,
        _: &T,
        _: &mut super::internal::parser::PreparedObjectAssembly,
        _: Option<&ParseContext>,
    ) -> Result<(), SchemaIssue> {
        Ok(())
    }
    fn properties(&self, _: &mut Vec<SchemaProperty>) -> Result<(), SchemaIssue> {
        Ok(())
    }
    fn validator_available(&self) -> bool {
        true
    }
    fn validate(&self, _: &T, _: Option<&ParseContext>) -> Result<(), SchemaIssue> {
        Ok(())
    }
    fn writer_available(&self) -> bool {
        true
    }
    fn write(&self, _: &T, _: &mut JsonWriter) -> Result<(), SchemaIssue> {
        Ok(())
    }
}
type Getter<T, F> = Arc<dyn for<'a> Fn(&'a T) -> &'a F + Send + Sync>;
type Omit<F> = Arc<dyn Fn(&F) -> bool + Send + Sync>;
/// One typed managed record field.
pub struct PreparedRecordField<T, F, R> {
    name: Arc<str>,
    codec: PureCodec<F, SharedPreparedJsonValue>,
    getter: Getter<T, F>,
    missing: Option<Arc<dyn Fn() -> F + Send + Sync>>,
    omit: Option<Omit<F>>,
    default: Option<Arc<JsonValue>>,
    rest: R,
}
impl<T: 'static, F: Send + 'static, R: PreparedRecordFields<T>> PreparedRecordFields<T>
    for PreparedRecordField<T, F, R>
{
    type Values = (F, R::Values);
    fn decode(
        &self,
        input: &SharedPreparedJsonValue,
        issues: &mut PreparedIssueAccumulator<'_>,
        context: Option<&ParseContext>,
    ) -> Result<Option<Self::Values>, SchemaIssue> {
        if issues.is_stopped() {
            return Ok(None);
        }
        let result = (|| {
            let encoded = input
                .try_get(&self.name)
                .map_err(|e| SchemaIssue::new(SchemaIssueKind::Memory(e)))?;
            match encoded {
                Some(encoded) => {
                    let child = super::parse_context::child(
                        context,
                        super::IssueSegment::Field(Arc::clone(&self.name)),
                    )?;
                    self.codec.decode_in_ref(&encoded, child.as_ref())
                }
                None => match &self.missing {
                    Some(factory) => {
                        let value = factory();
                        if self.default.is_some() {
                            let child = super::parse_context::child(
                                context,
                                super::IssueSegment::Field(Arc::clone(&self.name)),
                            )?;
                            self.codec.validate_in_ref(&value, child.as_ref())?;
                        }
                        Ok(value)
                    }
                    None => Err(SchemaIssue::new(SchemaIssueKind::MissingField(Arc::clone(
                        &self.name,
                    )))),
                },
            }
        })()
        .map_err(|e: SchemaIssue| {
            super::prepared_issue_at(
                context,
                e,
                super::IssueSegment::Field(Arc::clone(&self.name)),
            )
        });
        let value = match result {
            Ok(value) => Some(value),
            Err(issue) => {
                issues.record(issue)?;
                None
            }
        };
        if issues.is_stopped() {
            return Ok(None);
        }
        let rest = self.rest.decode(input, issues, context)?;
        Ok(value.zip(rest))
    }
    fn encode(
        &self,
        value: &T,
        output: &mut super::internal::parser::PreparedObjectAssembly,
        context: Option<&ParseContext>,
    ) -> Result<(), SchemaIssue> {
        let field = (self.getter)(value);
        if !self.omit.as_ref().is_some_and(|f| f(field)) {
            output.insert(&self.name, || {
                let child = super::parse_context::child(
                    context,
                    super::IssueSegment::Field(Arc::clone(&self.name)),
                )?;
                self.codec
                    .encode_in_ref(field, child.as_ref())
                    .map_err(|e| {
                        super::prepared_issue_at(
                            context,
                            e,
                            super::IssueSegment::Field(Arc::clone(&self.name)),
                        )
                    })
            })?;
        }
        self.rest.encode(value, output, context)
    }
    fn properties(&self, out: &mut Vec<SchemaProperty>) -> Result<(), SchemaIssue> {
        out.push(SchemaProperty::new(
            Arc::clone(&self.name),
            self.codec.ast().clone(),
            self.missing.is_none(),
            self.default.as_ref().map(Arc::clone),
        )?);
        self.rest.properties(out)
    }
    fn validator_available(&self) -> bool {
        self.codec.has_type_validator() && self.rest.validator_available()
    }
    fn validate(&self, value: &T, context: Option<&ParseContext>) -> Result<(), SchemaIssue> {
        let field = (self.getter)(value);
        if !self.omit.as_ref().is_some_and(|f| f(field)) {
            let child = super::parse_context::child(
                context,
                super::IssueSegment::Field(Arc::clone(&self.name)),
            )?;
            self.codec
                .validate_in_ref(field, child.as_ref())
                .map_err(|e| {
                    super::prepared_issue_at(
                        context,
                        e,
                        super::IssueSegment::Field(Arc::clone(&self.name)),
                    )
                })?;
        }
        self.rest.validate(value, context)
    }
    fn writer_available(&self) -> bool {
        self.codec.has_writer() && self.rest.writer_available()
    }
    fn write(&self, value: &T, writer: &mut JsonWriter) -> Result<(), SchemaIssue> {
        let field = (self.getter)(value);
        if !self.omit.as_ref().is_some_and(|f| f(field)) {
            writer.property(&self.name)?;
            self.codec.write_into(field, writer).map_err(|e| {
                writer.issue_at(e, super::IssueSegment::Field(Arc::clone(&self.name)))
            })?;
        }
        self.rest.write(value, writer)
    }
}
/// Builder for a pure codec over managed JSON documents.
pub struct PreparedRecordBuilder<T, Fields = PreparedRecordNil> {
    fields: Fields,
    marker: PhantomData<fn() -> T>,
}
impl<T> PreparedRecordBuilder<T, PreparedRecordNil> {
    /// Create an empty typed definition.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            fields: PreparedRecordNil,
            marker: PhantomData,
        }
    }
}
impl<T> Default for PreparedRecordBuilder<T, PreparedRecordNil> {
    fn default() -> Self {
        Self {
            fields: PreparedRecordNil,
            marker: PhantomData,
        }
    }
}
impl<T, Fields> PreparedRecordBuilder<T, Fields> {
    /// Add a required field.
    pub fn field<F: Send + 'static>(
        self,
        name: impl Into<Arc<str>>,
        getter: impl for<'a> Fn(&'a T) -> &'a F + Send + Sync + 'static,
        codec: PureCodec<F, SharedPreparedJsonValue>,
    ) -> PreparedRecordBuilder<T, PreparedRecordField<T, F, Fields>> {
        self.add(name, getter, codec, None, None, None)
    }
    /// Add a field with an absence factory and omission predicate.
    pub fn optional<F: Send + 'static>(
        self,
        name: impl Into<Arc<str>>,
        getter: impl for<'a> Fn(&'a T) -> &'a F + Send + Sync + 'static,
        codec: PureCodec<F, SharedPreparedJsonValue>,
        missing: impl Fn() -> F + Send + Sync + 'static,
        omit: impl Fn(&F) -> bool + Send + Sync + 'static,
    ) -> PreparedRecordBuilder<T, PreparedRecordField<T, F, Fields>> {
        self.add(
            name,
            getter,
            codec,
            Some(Arc::new(missing)),
            Some(Arc::new(omit)),
            None,
        )
    }
    /// Add an optional field with its independently supplied encoded default metadata.
    pub fn optional_with_default<F: Send + 'static>(
        self,
        name: impl Into<Arc<str>>,
        getter: impl for<'a> Fn(&'a T) -> &'a F + Send + Sync + 'static,
        codec: PureCodec<F, SharedPreparedJsonValue>,
        missing: impl Fn() -> F + Send + Sync + 'static,
        omit: impl Fn(&F) -> bool + Send + Sync + 'static,
        default: Arc<JsonValue>,
    ) -> PreparedRecordBuilder<T, PreparedRecordField<T, F, Fields>> {
        self.add(
            name,
            getter,
            codec,
            Some(Arc::new(missing)),
            Some(Arc::new(omit)),
            Some(default),
        )
    }
    fn add<F: Send + 'static>(
        self,
        name: impl Into<Arc<str>>,
        getter: impl for<'a> Fn(&'a T) -> &'a F + Send + Sync + 'static,
        codec: PureCodec<F, SharedPreparedJsonValue>,
        missing: Option<Arc<dyn Fn() -> F + Send + Sync>>,
        omit: Option<Omit<F>>,
        default: Option<Arc<JsonValue>>,
    ) -> PreparedRecordBuilder<T, PreparedRecordField<T, F, Fields>> {
        PreparedRecordBuilder {
            fields: PreparedRecordField {
                name: name.into(),
                codec,
                getter: Arc::new(getter),
                missing,
                omit,
                default,
                rest: self.fields,
            },
            marker: PhantomData,
        }
    }
}
impl<T: Send + 'static, Fields: PreparedRecordFields<T>> PreparedRecordBuilder<T, Fields> {
    /// Finish the definition with explicit output, diagnostic and traversal backing.
    /// Rejecting unknown fields in accumulate mode requires one physical issue
    /// entry beyond the logical issue limit for the overflow-detecting input.
    pub fn finish(
        self,
        policy: ParsePolicy,
        output: Arc<PreparedJsonParserPool>,
        issues: impl Into<PreparedRecordIssues>,
        validation: Arc<PreparedJsonValidationPool>,
        construct: impl Fn(Fields::Values) -> T + Send + Sync + 'static,
    ) -> Result<PureCodec<T, SharedPreparedJsonValue>, SchemaIssue> {
        let issues = issues.into();
        let mut properties = Vec::new();
        self.fields.properties(&mut properties)?;
        let known: BTreeSet<Arc<str>> = properties.iter().map(|p| Arc::from(p.name())).collect();
        let ast = SchemaAst::object(properties, policy.unknown_fields)?;
        let fields = Arc::new(self.fields);
        let decoder = Arc::clone(&fields);
        let validator = Arc::clone(&fields);
        let writer = Arc::clone(&fields);
        let has_validator = fields.validator_available();
        let has_writer = fields.writer_available();
        let codec = PureCodec::with_context_ast(
            ast,
            move |input, context| {
                let mut active = super::parse_context::policy(context, policy);
                active.unknown_fields = policy.unknown_fields;
                validation.check_parse_limits(input, &active.limits)?;
                if !matches!(input.view().kind(), Some(PreparedJsonKind::Object)) {
                    return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                        expected: ValueKind::Record,
                        actual: super::json_codec::prepared_value_kind(input)?,
                    }));
                }
                let mut collected =
                    PreparedIssueAccumulator::admit(issues.pool(), &validation, active)
                        .map_err(|e| SchemaIssue::new(SchemaIssueKind::Memory(e)))?;
                if active.unknown_fields == UnknownFields::Reject
                    && active.errors == super::SchemaErrorMode::Accumulate
                {
                    let required = active.limits.issues.get().checked_add(1).ok_or_else(|| {
                        SchemaIssue::new(SchemaIssueKind::Memory(
                            crate::memory::MemoryError::ArithmeticOverflow,
                        ))
                    })?;
                    collected
                        .require_entries(required)
                        .map_err(|e| SchemaIssue::new(SchemaIssueKind::Memory(e)))?;
                }
                if active.unknown_fields == UnknownFields::Reject {
                    let mut current = input.view().first_child();
                    while let Some(child) = current {
                        let name = child
                            .property_name()
                            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                        if !known.contains(name) {
                            collected.record_unknown(super::prepared_unknown_field(
                                context,
                                issues.issue_memory(),
                                name,
                            ))?;
                            if collected.unknown_stopped() {
                                break;
                            }
                        }
                        current = child.next_sibling();
                    }
                }
                collected.begin_fields();
                let values = if collected.is_stopped() {
                    None
                } else {
                    decoder.decode(input, &mut collected, context)?
                };
                if let Some(issue) = collected.finish() {
                    Err(issue)
                } else if let Some(values) = values {
                    Ok(construct(values))
                } else {
                    Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition))
                }
            },
            move |value, context| {
                let policy = super::parse_context::policy(context, policy);
                let mut object = output.admit_object(policy)?;
                fields.encode(value, &mut object, context)?;
                object.finish()
            },
        );
        let codec = if has_validator {
            codec.with_context_type_validator(move |value, context| {
                validator.validate(value, context)
            })
        } else {
            codec
        };
        Ok(if has_writer {
            codec.with_writer(move |value, output| {
                output.begin_object()?;
                writer.write(value, output)?;
                output.end_object()
            })
        } else {
            codec
        })
    }
}
