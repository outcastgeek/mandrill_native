//! Managed representation backend for tagged pure enum codecs.
use super::{
    IssueSegment, JsonValue, JsonWriter, ParseContext, ParsePolicy, PreparedJsonKind,
    PreparedJsonParserPool, PreparedJsonValidationPool, PureCodec, SchemaAst, SchemaIssue,
    SchemaIssueKind, SchemaProperty, SchemaShape, SharedPreparedJsonValue, UnknownFields,
    ValueKind,
};
use std::{collections::BTreeSet, sync::Arc};
type Decode<T> = Arc<
    dyn Fn(&SharedPreparedJsonValue, Option<&ParseContext>) -> Result<T, SchemaIssue> + Send + Sync,
>;
type Encode<T> = Arc<
    dyn Fn(&T, Option<&ParseContext>) -> Option<Result<SharedPreparedJsonValue, SchemaIssue>>
        + Send
        + Sync,
>;
type Validate<T> = Arc<dyn Fn(&T, Option<&ParseContext>) -> Result<(), SchemaIssue> + Send + Sync>;
type Write<T> = Arc<dyn Fn(&T, &mut JsonWriter) -> Result<(), SchemaIssue> + Send + Sync>;
struct Variant<T> {
    tag: Arc<str>,
    schema: SchemaAst,
    decode: Decode<T>,
    matches: Arc<dyn Fn(&T) -> bool + Send + Sync>,
    encode: Encode<T>,
    validate: Option<Validate<T>>,
    writer: Option<Write<T>>,
}
/// Tagged managed JSON codec definition. Definition metadata uses startup native storage.
/// Encoded objects retain tag-then-content insertion order; ordinary serde maps
/// can serialize these fields in key order.
pub struct PreparedTaggedEnumBuilder<T> {
    tag: Arc<str>,
    content: Arc<str>,
    variants: Vec<Variant<T>>,
    tags: BTreeSet<Arc<str>>,
}
impl<T: Send + 'static> PreparedTaggedEnumBuilder<T> {
    /// Construct an empty definition with explicit discriminator and content names.
    #[must_use]
    pub fn new(tag: impl Into<Arc<str>>, content: impl Into<Arc<str>>) -> Self {
        Self {
            tag: tag.into(),
            content: content.into(),
            variants: Vec::new(),
            tags: BTreeSet::new(),
        }
    }
    /// Add one typed managed payload variant.
    pub fn variant<P: Send + 'static>(
        mut self,
        tag: impl Into<Arc<str>>,
        codec: PureCodec<P, SharedPreparedJsonValue>,
        project: impl for<'a> Fn(&'a T) -> Option<&'a P> + Send + Sync + 'static,
        construct: impl Fn(P) -> T + Send + Sync + 'static,
    ) -> Result<Self, SchemaIssue> {
        let tag = tag.into();
        if tag.is_empty() || !self.tags.insert(Arc::clone(&tag)) {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        let schema = codec.ast().clone();
        let decoder = codec.clone();
        let project = Arc::new(project);
        let matcher = Arc::clone(&project);
        let writer = if codec.has_writer() {
            let codec = codec.clone();
            let project = Arc::clone(&project);
            Some(Arc::new(move |v: &T, w: &mut JsonWriter| {
                codec.write_into(
                    project(v).ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidTag))?,
                    w,
                )
            }) as Write<T>)
        } else {
            None
        };
        let validate = if codec.has_type_validator() {
            let codec = codec.clone();
            let project = Arc::clone(&project);
            Some(Arc::new(move |v: &T, c: Option<&ParseContext>| {
                codec.validate_in_ref(
                    project(v).ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidTag))?,
                    c,
                )
            }) as Validate<T>)
        } else {
            None
        };
        self.variants.push(Variant {
            tag,
            schema,
            decode: Arc::new(move |v, c| decoder.decode_in_ref(v, c).map(&construct)),
            matches: Arc::new(move |v| matcher(v).is_some()),
            encode: Arc::new(move |v, c| project(v).map(|p| codec.encode_in_ref(p, c))),
            validate,
            writer,
        });
        Ok(self)
    }
    /// Finish with caller-selected output and validation backing.
    pub fn finish(
        self,
        policy: ParsePolicy,
        output: Arc<PreparedJsonParserPool>,
        validation: Arc<PreparedJsonValidationPool>,
    ) -> Result<PureCodec<T, SharedPreparedJsonValue>, SchemaIssue> {
        if self.tag.is_empty()
            || self.content.is_empty()
            || self.tag == self.content
            || self.variants.is_empty()
        {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        let mut shapes = Vec::with_capacity(self.variants.len());
        for variant in &self.variants {
            let tag = Arc::new(JsonValue::String(variant.tag.to_string()));
            let a = SchemaProperty::new(
                Arc::clone(&self.tag),
                SchemaAst::new(SchemaShape::Literal(tag)),
                true,
                None,
            )?;
            let b = SchemaProperty::new(
                Arc::clone(&self.content),
                variant.schema.clone(),
                true,
                None,
            )?;
            shapes.push(SchemaAst::object(vec![a, b], policy.unknown_fields)?);
        }
        let ast = SchemaAst::new(SchemaShape::Union(shapes))
            .annotate("discriminant", Arc::clone(&self.tag))?;
        let variants = Arc::new(self.variants);
        let dec = Arc::clone(&variants);
        let valid = Arc::clone(&variants);
        let write = Arc::clone(&variants);
        let has_validator = variants.iter().all(|v| v.validate.is_some());
        let has_writer = variants.iter().all(|v| v.writer.is_some());
        let wt = Arc::clone(&self.tag);
        let wc = Arc::clone(&self.content);
        let dt = Arc::clone(&self.tag);
        let dc = Arc::clone(&self.content);
        let tag = self.tag;
        let content = self.content;
        let decode_output = Arc::clone(&output);
        let codec = PureCodec::with_context_ast(
            ast,
            move |input: &SharedPreparedJsonValue, context| {
                let mut active = super::parse_context::policy(context, policy);
                active.unknown_fields = policy.unknown_fields;
                validation.check_parse_limits(input, &active.limits)?;
                let root = input.view();
                if !matches!(root.kind(), Some(PreparedJsonKind::Object)) {
                    return Err(SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                        expected: ValueKind::Record,
                        actual: super::json_codec::prepared_value_kind(input)?,
                    }));
                }
                if active.unknown_fields == UnknownFields::Reject {
                    let mut cursor = root.first_child();
                    while let Some(item) = cursor {
                        let key = item
                            .property_name()
                            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                        if key != dt.as_ref() && key != dc.as_ref() {
                            return Err(super::prepared_unknown_field(
                                context,
                                decode_output.issue_memory(),
                                key,
                            ));
                        }
                        cursor = item.next_sibling();
                    }
                }
                let tag_value = root.get(&dt).ok_or_else(|| {
                    SchemaIssue::new(SchemaIssueKind::MissingField(Arc::clone(&dt)))
                })?;
                let tag =
                    tag_value.as_str().ok_or_else(
                        || match super::json_codec::prepared_view_kind(tag_value) {
                            Ok(actual) => super::prepared_issue_at(
                                context,
                                SchemaIssue::new(SchemaIssueKind::TypeMismatch {
                                    expected: ValueKind::String,
                                    actual,
                                }),
                                IssueSegment::Field(Arc::clone(&dt)),
                            ),
                            Err(issue) => issue,
                        },
                    )?;
                let variant = dec.iter().find(|v| v.tag.as_ref() == tag).ok_or_else(|| {
                    super::prepared_issue_at(
                        context,
                        SchemaIssue::new(SchemaIssueKind::InvalidTag),
                        IssueSegment::Field(Arc::clone(&dt)),
                    )
                })?;
                let content = input
                    .try_get(&dc)
                    .map_err(|e| SchemaIssue::new(SchemaIssueKind::Memory(e)))?
                    .ok_or_else(|| {
                        SchemaIssue::new(SchemaIssueKind::MissingField(Arc::clone(&dc)))
                    })?;
                (variant.decode)(
                    &content,
                    super::parse_context::child(context, IssueSegment::Field(Arc::clone(&dc)))?
                        .as_ref(),
                )
                .map_err(|issue| {
                    super::prepared_issue_at(
                        context,
                        super::prepared_issue_at(
                            context,
                            issue,
                            IssueSegment::Field(Arc::clone(&dc)),
                        ),
                        IssueSegment::Variant(Arc::clone(&variant.tag)),
                    )
                })
            },
            move |value: &T, context| {
                let active = super::parse_context::policy(context, policy);
                let mut object = output.admit_object(active)?;
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
                object.insert_string(&tag, &variant.tag)?;
                object.insert(&content, || {
                    (variant.encode)(
                        value,
                        super::parse_context::child(
                            context,
                            IssueSegment::Field(Arc::clone(&content)),
                        )?
                        .as_ref(),
                    )
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?
                    .map_err(|issue| {
                        super::prepared_issue_at(
                            context,
                            super::prepared_issue_at(
                                context,
                                issue,
                                IssueSegment::Field(Arc::clone(&content)),
                            ),
                            IssueSegment::Variant(Arc::clone(&variant.tag)),
                        )
                    })
                })?;
                object.finish()
            },
        );
        let codec = if has_validator {
            codec.with_context_type_validator(move |value, context| {
                let mut matched = valid.iter().filter(|v| (v.matches)(value));
                let variant = matched
                    .next()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidTag))?;
                if matched.next().is_some() {
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
                .map_err(|issue| {
                    super::prepared_issue_at(
                        context,
                        issue,
                        IssueSegment::Variant(Arc::clone(&variant.tag)),
                    )
                })
            })
        } else {
            codec
        };
        Ok(if has_writer {
            codec.with_writer(move |value, writer| {
                let mut matched = write.iter().filter(|v| (v.matches)(value));
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
                writer.property(&wt)?;
                writer.string(&variant.tag)?;
                writer.property(&wc)?;
                write(value, writer)?;
                writer.end_object()
            })
        } else {
            codec
        })
    }
}
