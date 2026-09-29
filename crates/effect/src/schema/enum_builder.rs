use super::{
    IssueSegment, JsonValue, ParsePolicy, PureCodec, SchemaAst, SchemaIssue, SchemaIssueKind,
    SchemaProperty, SchemaShape, UnknownFields, ValueKind, value_kind,
};
use std::collections::BTreeSet;
use std::sync::Arc;

type VariantDecode<T> = Arc<dyn Fn(&JsonValue) -> Result<T, SchemaIssue> + Send + Sync>;
type VariantEncode<T> = Arc<dyn Fn(&T) -> Option<Result<JsonValue, SchemaIssue>> + Send + Sync>;

struct Variant<T> {
    tag: Arc<str>,
    schema: SchemaAst,
    decode: VariantDecode<T>,
    matches: Arc<dyn Fn(&T) -> bool + Send + Sync>,
    encode: VariantEncode<T>,
}

/// Typed registration keeps each payload codec coupled to its own projector
/// and constructor. Only the common JSON envelope is dispatched dynamically.
pub struct TaggedEnumBuilder<T> {
    tag_field: Arc<str>,
    content_field: Arc<str>,
    variants: Vec<Variant<T>>,
    tags: BTreeSet<Arc<str>>,
}

impl<T: Send + 'static> TaggedEnumBuilder<T> {
    #[must_use]
    pub fn new(tag_field: impl Into<Arc<str>>, content_field: impl Into<Arc<str>>) -> Self {
        Self {
            tag_field: tag_field.into(),
            content_field: content_field.into(),
            variants: Vec::new(),
            tags: BTreeSet::new(),
        }
    }

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
        self.variants.push(Variant {
            tag,
            schema,
            decode: Arc::new(move |payload| decode.decode_ref(payload).map(&construct)),
            matches: Arc::new(move |value| matcher(value).is_some()),
            encode: Arc::new(move |value| project(value).map(|payload| codec.encode_ref(payload))),
        });
        Ok(self)
    }

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
        let ast = SchemaAst::new(SchemaShape::Union(shapes));
        let variants = Arc::new(self.variants);
        let decode_variants = Arc::clone(&variants);
        let tag_field = self.tag_field;
        let content_field = self.content_field;
        let decode_tag = Arc::clone(&tag_field);
        let decode_content = Arc::clone(&content_field);
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
                (variant.decode)(content).map_err(|issue| {
                    issue
                        .at(IssueSegment::Field(Arc::clone(&decode_content)))
                        .at(IssueSegment::Variant(Arc::clone(&variant.tag)))
                })
            },
            move |value| {
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
                let payload = (variant.encode)(value)
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
        ))
    }
}
