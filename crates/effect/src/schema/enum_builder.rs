//! Typed tagged-enum schema construction.

use super::{
    IssueSegment, JsonValue, ParsePolicy, PureCodec, SchemaAst, SchemaIssue, SchemaIssueKind,
    SchemaProperty, SchemaShape, UnknownFields, ValueKind, value_kind,
};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Type alias for variant decode in schema enum builder.
///
/// # Type parameters
/// - `T`: Value or item type.
type VariantDecode<T> =
    Arc<dyn Fn(&JsonValue, Option<&super::ParseContext>) -> Result<T, SchemaIssue> + Send + Sync>;
/// Type alias for variant encode in schema enum builder.
///
/// # Type parameters
/// - `T`: Value or item type.
type VariantEncode<T> = Arc<
    dyn Fn(&T, Option<&super::ParseContext>) -> Option<Result<JsonValue, SchemaIssue>>
        + Send
        + Sync,
>;

type VariantValidator<T> =
    Arc<dyn Fn(&T, Option<&super::ParseContext>) -> Result<(), SchemaIssue> + Send + Sync>;
type VariantWriter<T> =
    Arc<dyn Fn(&T, &mut super::JsonWriter) -> Result<(), SchemaIssue> + Send + Sync>;

/// Typed variant used by schema enum builder.
///
/// # Type parameters
/// - `T`: Value or item type.
struct Variant<T> {
    tag: Arc<str>,
    schema: SchemaAst,
    decode: VariantDecode<T>,
    matches: Arc<dyn Fn(&T) -> bool + Send + Sync>,
    encode: VariantEncode<T>,
    writer: Option<VariantWriter<T>>,
    validate: Option<VariantValidator<T>>,
}

/// A builder that registers tagged variants and their codecs before producing a checked schema.
/// and constructor. Only the common JSON envelope is dispatched dynamically.
///
/// # Type parameters
/// - `T`: Value or item type.
pub struct TaggedEnumBuilder<T> {
    tag_field: Arc<str>,
    content_field: Arc<str>,
    variants: Vec<Variant<T>>,
    tags: BTreeSet<Arc<str>>,
}

impl<T: Send + 'static> TaggedEnumBuilder<T> {
    #[must_use]
    /// Construct a `TaggedEnumBuilder` from its required inputs and explicit policies.
    ///
    /// # Type parameters
    /// - `T`: Value or item type. Bound: `Send + 'static`.
    ///
    /// # Parameters
    /// - `tag_field` (`impl Into<Arc<str>>`): The tag field supplied to this operation.
    /// - `content_field` (`impl Into<Arc<str>>`): The content field supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(tag_field: impl Into<Arc<str>>, content_field: impl Into<Arc<str>>) -> Self {
        Self {
            tag_field: tag_field.into(),
            content_field: content_field.into(),
            variants: Vec::new(),
            tags: BTreeSet::new(),
        }
    }

    /// Add a tag with its payload codec, owned constructor and borrowed variant projection.
    ///
    /// `project` must return Some only for this variant. Duplicate/empty tags fail
    /// construction. Payload validation and writing propagate only when the codec has
    /// genuine witnesses; projection never encodes a value merely to identify a tag.
    pub fn variant<P: Send + 'static>(
        mut self,
        tag: impl Into<Arc<str>>,
        codec: PureCodec<P, JsonValue>,
        project: impl for<'a> Fn(&'a T) -> Option<&'a P> + Send + Sync + 'static,
        construct: impl Fn(P) -> T + Send + Sync + 'static,
    ) -> Result<Self, SchemaIssue> {
        let tag = tag.into();
        if tag.is_empty() || !self.tags.insert(Arc::clone(&tag)) {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        let schema = codec.ast().clone();
        let decode = codec.clone();
        let project = Arc::new(project);
        let matcher = Arc::clone(&project);
        let writer = if codec.has_writer() {
            let codec = codec.clone();
            let project = Arc::clone(&project);
            Some(Arc::new(move |value: &T, writer: &mut super::JsonWriter| {
                codec.write_into(
                    project(value).ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidTag))?,
                    writer,
                )
            })
                as Arc<
                    dyn Fn(&T, &mut super::JsonWriter) -> Result<(), SchemaIssue> + Send + Sync,
                >)
        } else {
            None
        };
        let validate = if codec.has_type_validator() {
            let codec = codec.clone();
            let project = Arc::clone(&project);
            Some(
                Arc::new(move |value: &T, context: Option<&super::ParseContext>| {
                    codec.validate_in_ref(
                        project(value)
                            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidTag))?,
                        context,
                    )
                }) as VariantValidator<T>,
            )
        } else {
            None
        };
        self.variants.push(Variant {
            tag,
            schema,
            writer,
            validate,
            decode: Arc::new(move |payload, context| {
                decode.decode_in_ref(payload, context).map(&construct)
            }),
            matches: Arc::new(move |value| matcher(value).is_some()),
            encode: Arc::new(move |value, context| {
                project(value).map(|payload| codec.encode_in_ref(payload, context))
            }),
        });
        Ok(self)
    }

    /// Prepare a tagged JSON object codec using the explicit parse policy.
    ///
    /// Rejects empty/equal tag and content field names or an empty variant list.
    /// Decode selects the tag before payload decoding; encode/typed validation reject
    /// zero or multiple matching projections. Independent validation or writing is
    /// available only if every variant supplies the corresponding payload witness.
    /// Discriminator metadata records the tag field without changing wire tags.
    pub fn finish(self, policy: ParsePolicy) -> Result<PureCodec<T, JsonValue>, SchemaIssue> {
        if self.tag_field.is_empty()
            || self.content_field.is_empty()
            || self.tag_field == self.content_field
            || self.variants.is_empty()
        {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        let mut shapes = Vec::with_capacity(self.variants.len());
        for variant in &self.variants {
            let tag = Arc::new(JsonValue::String(variant.tag.to_string()));
            let tag_property = SchemaProperty::new(
                Arc::clone(&self.tag_field),
                SchemaAst::new(SchemaShape::Literal(tag)),
                true,
                None,
            )?;
            let content_property = SchemaProperty::new(
                Arc::clone(&self.content_field),
                variant.schema.clone(),
                true,
                None,
            )?;
            shapes.push(SchemaAst::object(
                vec![tag_property, content_property],
                policy.unknown_fields,
            )?);
        }
        let ast = SchemaAst::new(SchemaShape::Union(shapes))
            .annotate("discriminant", Arc::clone(&self.tag_field))?;
        let variants = Arc::new(self.variants);
        let decode_variants = Arc::clone(&variants);
        let writer_variants = Arc::clone(&variants);
        let validation_variants = Arc::clone(&variants);
        let has_validator = variants.iter().all(|variant| variant.validate.is_some());
        let has_writer = variants.iter().all(|variant| variant.writer.is_some());
        let writer_tag = Arc::clone(&self.tag_field);
        let writer_content = Arc::clone(&self.content_field);
        let tag_field = self.tag_field;
        let content_field = self.content_field;
        let decode_tag = Arc::clone(&tag_field);
        let decode_content = Arc::clone(&content_field);
        let codec = PureCodec::with_context_ast(
            ast,
            move |encoded, context| {
                let mut active = super::parse_context::policy(context, policy);
                active.unknown_fields = policy.unknown_fields;
                let policy = active;
                super::internal::parser::check_limits(encoded, &policy.limits)?;
                let JsonValue::Object(object) = encoded else {
                    return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                        expected: ValueKind::Record,
                        actual: value_kind(encoded),
                    }));
                };
                if policy.unknown_fields == UnknownFields::Reject {
                    for key in object.keys() {
                        if key != decode_tag.as_ref() && key != decode_content.as_ref() {
                            return Err(SchemaIssue::new(SchemaIssueKind::UnknownField(
                                Arc::from(key.as_str()),
                            )));
                        }
                    }
                }
                let tag = object.get(decode_tag.as_ref()).ok_or_else(|| {
                    SchemaIssue::new(SchemaIssueKind::MissingField(Arc::clone(&decode_tag)))
                })?;
                let JsonValue::String(tag) = tag else {
                    return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                        expected: ValueKind::String,
                        actual: value_kind(tag),
                    })
                    .at(IssueSegment::Field(Arc::clone(&decode_tag))));
                };
                let variant = decode_variants
                    .iter()
                    .find(|variant| variant.tag.as_ref() == tag)
                    .ok_or_else(|| {
                        SchemaIssue::new(SchemaIssueKind::InvalidTag)
                            .at(IssueSegment::Field(Arc::clone(&decode_tag)))
                    })?;
                let content = object.get(decode_content.as_ref()).ok_or_else(|| {
                    SchemaIssue::new(SchemaIssueKind::MissingField(Arc::clone(&decode_content)))
                })?;
                (variant.decode)(
                    content,
                    super::parse_context::child(
                        context,
                        IssueSegment::Field(Arc::clone(&decode_content)),
                    )?
                    .as_ref(),
                )
                .map_err(|issue| {
                    issue
                        .at(IssueSegment::Field(Arc::clone(&decode_content)))
                        .at(IssueSegment::Variant(Arc::clone(&variant.tag)))
                })
            },
            move |value, context| {
                let policy = super::parse_context::policy(context, policy);
                let mut selected = None;
                for variant in variants.iter() {
                    if (variant.matches)(value) {
                        if selected.is_some() {
                            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
                        }
                        selected = Some(variant);
                    }
                }
                let variant =
                    selected.ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidTag))?;
                let payload = (variant.encode)(
                    value,
                    super::parse_context::child(
                        context,
                        IssueSegment::Field(Arc::clone(&content_field)),
                    )?
                    .as_ref(),
                )
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?
                .map_err(|issue| {
                    issue
                        .at(IssueSegment::Field(Arc::clone(&content_field)))
                        .at(IssueSegment::Variant(Arc::clone(&variant.tag)))
                })?;
                let mut object = serde_json::Map::new();
                object.insert(
                    tag_field.to_string(),
                    JsonValue::String(variant.tag.to_string()),
                );
                object.insert(content_field.to_string(), payload);
                let encoded = JsonValue::Object(object);
                super::internal::parser::check_limits(&encoded, &policy.limits)?;
                Ok(encoded)
            },
        );
        let codec = if has_validator {
            codec.with_context_type_validator(move |value, context| {
                let mut matches = validation_variants
                    .iter()
                    .filter(|variant| (variant.matches)(value));
                let variant = matches
                    .next()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidTag))?;
                if matches.next().is_some() {
                    return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
                }
                let child = super::parse_context::descend(context)?;
                variant
                    .validate
                    .as_ref()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::MissingValidator))?(
                    value,
                    child.as_ref(),
                )
                .map_err(|issue| issue.at(IssueSegment::Variant(Arc::clone(&variant.tag))))
            })
        } else {
            codec
        };
        Ok(if has_writer {
            codec.with_writer(move |value, writer| {
                let mut matched = writer_variants
                    .iter()
                    .filter(|variant| (variant.matches)(value));
                let variant = matched
                    .next()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidTag))?;
                if matched.next().is_some() {
                    return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
                }
                let write = variant
                    .writer
                    .as_ref()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                writer.begin_object()?;
                writer.property(&writer_tag)?;
                writer.string(&variant.tag)?;
                writer.property(&writer_content)?;
                write(value, writer)?;
                writer.end_object()
            })
        } else {
            codec
        })
    }
}
