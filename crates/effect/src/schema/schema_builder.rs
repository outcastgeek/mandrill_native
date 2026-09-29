//! Typed record fields and default construction.

use super::{
    JsonValue, ParsePolicy, PureCodec, SchemaAst, SchemaErrorMode, SchemaIssue, SchemaIssueKind,
    SchemaProperty, SchemaResource, UnknownFields, ValueKind, value_kind,
};
use std::collections::BTreeSet;
use std::sync::Arc;

mod sealed {
    /// Seals record-field lists to checked builder states.
    pub trait Sealed {}
    impl Sealed for super::RecordNil {}
    impl<T, F, Rest: Sealed> Sealed for super::RecordField<T, F, Rest> {}
}

/// A typed list of fields. Construction is sealed; consumers extend it only
/// through RecordBuilder::field/optional, which bind codecs to real getters.
pub trait RecordFields<T>: sealed::Sealed + Send + Sync + 'static {
    /// Typed values accumulated by the record builder.
    type Values: Send + 'static;
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `object` (`&serde_json::Map<String, JsonValue>`): Encoded object whose fields are validated.
    /// - `policy` (`ParsePolicy`): The caller-supplied behavior policy.
    /// - `issues` (`&mut Vec<SchemaIssue>`): The issues supplied to this operation.
    ///
    /// # Returns
    /// Type: `Option<Self::Values>`.
    /// Returns the value when present, or `None`.
    fn decode(
        &self,
        object: &serde_json::Map<String, JsonValue>,
        policy: ParsePolicy,
        issues: &mut Vec<SchemaIssue>,
    ) -> Option<Self::Values>;
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `value` (`&T`): Value read, transformed, encoded, or stored by this operation.
    /// - `object` (`&mut serde_json::Map<String, JsonValue>`): Encoded object whose fields are validated.
    ///
    /// # Returns
    /// Type: `Result<(), SchemaIssue>`.
    fn encode(
        &self,
        value: &T,
        object: &mut serde_json::Map<String, JsonValue>,
    ) -> Result<(), SchemaIssue>;
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `output` (`&mut Vec<SchemaProperty>`): The output supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), SchemaIssue>`.
    fn properties(&self, output: &mut Vec<SchemaProperty>) -> Result<(), SchemaIssue>;
}

/// The empty tail that terminates a typed record-field list.
pub struct RecordNil;

impl<T> RecordFields<T> for RecordNil {
    /// Type alias for values in schema schema builder.
    type Values = ();

    ///
    /// # Returns
    /// Type: `Option<()>`.
    /// Returns the value when present, or `None`.
    fn decode(
        &self,
        _: &serde_json::Map<String, JsonValue>,
        _: ParsePolicy,
        _: &mut Vec<SchemaIssue>,
    ) -> Option<()> {
        Some(())
    }

    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Result<(), SchemaIssue>`.
    fn encode(&self, _: &T, _: &mut serde_json::Map<String, JsonValue>) -> Result<(), SchemaIssue> {
        Ok(())
    }

    ///
    /// # Returns
    /// Type: `Result<(), SchemaIssue>`.
    fn properties(&self, _: &mut Vec<SchemaProperty>) -> Result<(), SchemaIssue> {
        Ok(())
    }
}

/// Type alias for getter in schema schema builder.
///
/// # Type parameters
/// - `T`: Value or item type.
/// - `F`: Callback or field-definition type.
type Getter<T, F> = Arc<dyn for<'a> Fn(&'a T) -> &'a F + Send + Sync>;
/// Type alias for missing in schema schema builder.
///
/// # Type parameters
/// - `F`: Callback or field-definition type.
type Missing<F> = Arc<dyn Fn() -> F + Send + Sync>;
/// Type alias for omit in schema schema builder.
///
/// # Type parameters
/// - `F`: Callback or field-definition type.
type Omit<F> = Arc<dyn Fn(&F) -> bool + Send + Sync>;

/// One named field codec and its typed place in a record schema.
///
/// # Type parameters
/// - `T`: Value or item type.
/// - `F`: Callback or field-definition type.
/// - `Rest`: Remaining typed record fields.
pub struct RecordField<T, F, Rest> {
    name: Arc<str>,
    codec: PureCodec<F, JsonValue>,
    getter: Getter<T, F>,
    missing: Option<Missing<F>>,
    omit: Option<Omit<F>>,
    pure_default: Option<Arc<JsonValue>>,
    rest: Rest,
}

impl<T: 'static, F: Send + 'static, Rest: RecordFields<T>> RecordFields<T>
    for RecordField<T, F, Rest>
{
    /// Type alias for values in schema schema builder.
    type Values = (F, Rest::Values);

    ///
    /// # Parameters
    /// - `object` (`&serde_json::Map<String, JsonValue>`): Encoded object whose fields are validated.
    /// - `policy` (`ParsePolicy`): The caller-supplied behavior policy.
    /// - `issues` (`&mut Vec<SchemaIssue>`): The issues supplied to this operation.
    ///
    /// # Returns
    /// Type: `Option<Self::Values>`.
    /// Returns the value when present, or `None`.
    fn decode(
        &self,
        object: &serde_json::Map<String, JsonValue>,
        policy: ParsePolicy,
        issues: &mut Vec<SchemaIssue>,
    ) -> Option<Self::Values> {
        let field = match object.get(self.name.as_ref()) {
            Some(encoded) => self.codec.decode_ref(encoded),
            None => match &self.missing {
                Some(factory) => Ok(factory()),
                None => Err(SchemaIssue::new(SchemaIssueKind::MissingField(Arc::clone(
                    &self.name,
                )))),
            },
        }
        .map_err(|issue| issue.at(super::IssueSegment::Field(Arc::clone(&self.name))));
        let field = match field {
            Ok(value) => Some(value),
            Err(issue) => {
                record_issue(issues, issue, policy);
                None
            }
        };
        if !issues.is_empty() && policy.errors == SchemaErrorMode::First {
            return None;
        }
        let rest = self.rest.decode(object, policy, issues);
        field.zip(rest)
    }

    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `value` (`&T`): Value read, transformed, encoded, or stored by this operation.
    /// - `object` (`&mut serde_json::Map<String, JsonValue>`): Encoded object whose fields are validated.
    ///
    /// # Returns
    /// Type: `Result<(), SchemaIssue>`.
    fn encode(
        &self,
        value: &T,
        object: &mut serde_json::Map<String, JsonValue>,
    ) -> Result<(), SchemaIssue> {
        let field = (self.getter)(value);
        if !self.omit.as_ref().is_some_and(|omit| omit(field)) {
            let encoded = self
                .codec
                .encode_ref(field)
                .map_err(|issue| issue.at(super::IssueSegment::Field(Arc::clone(&self.name))))?;
            object.insert(self.name.to_string(), encoded);
        }
        self.rest.encode(value, object)
    }

    ///
    /// # Parameters
    /// - `output` (`&mut Vec<SchemaProperty>`): The output supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<(), SchemaIssue>`.
    fn properties(&self, output: &mut Vec<SchemaProperty>) -> Result<(), SchemaIssue> {
        if let Some(default) = &self.pure_default {
            self.codec
                .decode_ref(default)
                .map_err(|issue| issue.at(super::IssueSegment::Field(Arc::clone(&self.name))))?;
        }
        output.push(SchemaProperty::new(
            Arc::clone(&self.name),
            self.codec.ast().clone(),
            self.missing.is_none(),
            self.pure_default.as_ref().map(Arc::clone),
        )?);
        self.rest.properties(output)
    }
}

/// Each added field changes Fields::Values. Finish requires a constructor for
/// exactly those values, so an incomplete definition cannot become a codec.
///
/// # Type parameters
/// - `T`: Value or item type.
/// - `Fields`: Typed list of record fields.
pub struct RecordBuilder<T, Fields = RecordNil> {
    fields: Fields,
    record: std::marker::PhantomData<fn() -> T>,
}

impl<T> RecordBuilder<T, RecordNil> {
    #[must_use]
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new() -> Self {
        Self {
            fields: RecordNil,
            record: std::marker::PhantomData,
        }
    }
}

impl<T> Default for RecordBuilder<T, RecordNil> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn default() -> Self {
        Self::new()
    }
}

impl<T, Fields> RecordBuilder<T, Fields> {
    #[must_use]
    /// Register a required named field and its codec in a record schema.
    ///
    /// # Type parameters
    /// - `Fields`: Typed list of record fields.
    /// - `F`: Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    /// - `getter` (`impl for<'a> Fn(&'a T) -> &'a F + Send + Sync + 'static`): The getter supplied to this operation.
    /// - `codec` (`PureCodec<F, JsonValue>`): The codec used to convert and validate values.
    ///
    /// # Returns
    /// Type: `RecordBuilder<T, RecordField<T, F, Fields>>`.
    pub fn field<F: Send + 'static>(
        self,
        name: impl Into<Arc<str>>,
        getter: impl for<'a> Fn(&'a T) -> &'a F + Send + Sync + 'static,
        codec: PureCodec<F, JsonValue>,
    ) -> RecordBuilder<T, RecordField<T, F, Fields>> {
        RecordBuilder {
            fields: RecordField {
                name: name.into(),
                codec,
                getter: Arc::new(getter),
                missing: None,
                omit: None,
                pure_default: None,
                rest: self.fields,
            },
            record: std::marker::PhantomData,
        }
    }

    #[must_use]
    /// Register an optional field with an explicit absence representation.
    ///
    /// # Type parameters
    /// - `Fields`: Typed list of record fields.
    /// - `F`: Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    /// - `getter` (`impl for<'a> Fn(&'a T) -> &'a F + Send + Sync + 'static`): The getter supplied to this operation.
    /// - `codec` (`PureCodec<F, JsonValue>`): The codec used to convert and validate values.
    /// - `missing` (`impl Fn() -> F + Send + Sync + 'static`): The missing supplied to this operation.
    /// - `omit` (`impl Fn(&F) -> bool + Send + Sync + 'static`): The omit supplied to this operation.
    ///
    /// # Returns
    /// Type: `RecordBuilder<T, RecordField<T, F, Fields>>`.
    pub fn optional<F: Send + 'static>(
        self,
        name: impl Into<Arc<str>>,
        getter: impl for<'a> Fn(&'a T) -> &'a F + Send + Sync + 'static,
        codec: PureCodec<F, JsonValue>,
        missing: impl Fn() -> F + Send + Sync + 'static,
        omit: impl Fn(&F) -> bool + Send + Sync + 'static,
    ) -> RecordBuilder<T, RecordField<T, F, Fields>> {
        RecordBuilder {
            fields: RecordField {
                name: name.into(),
                codec,
                getter: Arc::new(getter),
                missing: Some(Arc::new(missing)),
                omit: Some(Arc::new(omit)),
                pure_default: None,
                rest: self.fields,
            },
            record: std::marker::PhantomData,
        }
    }

    /// Metadata describes a pure encoded default; the factory constructs a
    /// fresh typed value for each missing field during decoding.
    #[must_use]
    /// # Type parameters
    /// - `Fields`: Typed list of record fields.
    /// - `F`: Bounds: `Send + 'static`.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    /// - `getter` (`impl for<'a> Fn(&'a T) -> &'a F + Send + Sync + 'static`): The getter supplied to this operation.
    /// - `codec` (`PureCodec<F, JsonValue>`): The codec used to convert and validate values.
    /// - `missing` (`impl Fn() -> F + Send + Sync + 'static`): The missing supplied to this operation.
    /// - `omit` (`impl Fn(&F) -> bool + Send + Sync + 'static`): The omit supplied to this operation.
    /// - `pure_default` (`Arc<JsonValue>`): The pure default supplied to this operation.
    ///
    /// # Returns
    /// Type: `RecordBuilder<T, RecordField<T, F, Fields>>`.
    pub fn optional_with_default<F: Send + 'static>(
        self,
        name: impl Into<Arc<str>>,
        getter: impl for<'a> Fn(&'a T) -> &'a F + Send + Sync + 'static,
        codec: PureCodec<F, JsonValue>,
        missing: impl Fn() -> F + Send + Sync + 'static,
        omit: impl Fn(&F) -> bool + Send + Sync + 'static,
        pure_default: Arc<JsonValue>,
    ) -> RecordBuilder<T, RecordField<T, F, Fields>> {
        RecordBuilder {
            fields: RecordField {
                name: name.into(),
                codec,
                getter: Arc::new(getter),
                missing: Some(Arc::new(missing)),
                omit: Some(Arc::new(omit)),
                pure_default: Some(pure_default),
                rest: self.fields,
            },
            record: std::marker::PhantomData,
        }
    }
}

impl<T: Send + 'static, Fields: RecordFields<T>> RecordBuilder<T, Fields> {
    /// Record the terminal status and release remaining operation ownership.
    ///
    /// # Type parameters
    /// - `Fields`: Typed list of record fields. Bound: `RecordFields<T>`.
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `policy` (`ParsePolicy`): The caller-supplied behavior policy.
    /// - `construct` (`impl Fn(Fields::Values) -> T + Send + Sync + 'static`): The construct supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<PureCodec<T, JsonValue>, SchemaIssue>`.
    pub fn finish(
        self,
        policy: ParsePolicy,
        construct: impl Fn(Fields::Values) -> T + Send + Sync + 'static,
    ) -> Result<PureCodec<T, JsonValue>, SchemaIssue> {
        let mut properties = Vec::new();
        self.fields.properties(&mut properties)?;
        let known: BTreeSet<_> = properties
            .iter()
            .map(|property| Arc::<str>::from(property.name()))
            .collect();
        let ast = SchemaAst::object(properties, policy.unknown_fields)?;
        let fields = Arc::new(self.fields);
        let decode = Arc::clone(&fields);
        Ok(PureCodec::with_ast(
            ast,
            move |encoded| {
                super::internal::parser::check_limits(encoded, &policy.limits)?;
                let JsonValue::Object(object) = encoded else {
                    return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                        expected: ValueKind::Record,
                        actual: value_kind(encoded),
                    }));
                };
                let mut issues = Vec::new();
                if policy.unknown_fields == UnknownFields::Reject {
                    for key in object.keys() {
                        if !known.contains(key.as_str()) {
                            record_issue(
                                &mut issues,
                                SchemaIssue::new(SchemaIssueKind::UnknownField(Arc::from(
                                    key.as_str(),
                                ))),
                                policy,
                            );
                            if policy.errors == SchemaErrorMode::First {
                                break;
                            }
                        }
                    }
                }
                let values = if policy.errors == SchemaErrorMode::First && !issues.is_empty() {
                    None
                } else {
                    decode.decode(object, policy, &mut issues)
                };
                if issues.is_empty()
                    && let Some(values) = values
                {
                    return Ok(construct(values));
                }
                if issues.len() == 1 {
                    return Err(issues.remove(0));
                }
                Err(SchemaIssue::new(SchemaIssueKind::Multiple).with_children(issues))
            },
            move |value| {
                let mut object = serde_json::Map::new();
                fields.encode(value, &mut object)?;
                let encoded = JsonValue::Object(object);
                super::internal::parser::check_limits(&encoded, &policy.limits)?;
                Ok(encoded)
            },
        ))
    }
}

///
/// # Parameters
/// - `issues` (`&mut Vec<SchemaIssue>`): The issues supplied to this operation.
/// - `issue` (`SchemaIssue`): The issue supplied to this operation.
/// - `policy` (`ParsePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `()`.
fn record_issue(issues: &mut Vec<SchemaIssue>, issue: SchemaIssue, policy: ParsePolicy) {
    if issues.len() == policy.limits.issues.get() {
        issues.clear();
        issues.push(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
            resource: SchemaResource::Issues,
        }));
    } else if !matches!(
        issues.first().map(SchemaIssue::kind),
        Some(SchemaIssueKind::LimitExceeded {
            resource: SchemaResource::Issues
        })
    ) {
        issues.push(issue);
    }
}
