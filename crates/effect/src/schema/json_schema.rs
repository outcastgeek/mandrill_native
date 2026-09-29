//! Encoded-side JSON Schema representation export.

use super::{
    JsonValue, PrimitiveShape, SchemaAst, SchemaShape, StringLengthUnit, StringPattern,
    UnknownFields,
};
use serde_json::{Map, Number};
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// The exporter uses caller-selected limits; sharing a schema node does not
/// exempt each traversal occurrence from the node/depth budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepresentationLimits {
    /// Maximum permitted nesting depth. Type: `NonZeroUsize`.
    pub depth: NonZeroUsize,
    /// Maximum permitted schema or parse nodes. Type: `NonZeroUsize`.
    pub nodes: NonZeroUsize,
    /// Maximum number of schema definitions in exported output. Type: `NonZeroUsize`.
    pub definitions: NonZeroUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Whether an exported representation preserves the source schema exactly or approximately.
pub enum RepresentationPrecision {
    /// The exported representation preserves the source constraint exactly.
    Exact,
    /// The exported representation may lose source-schema constraints.
    Approximate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A typed reason a schema cannot be exported under the requested representation limits.
pub enum RepresentationError {
    /// The configured depth limit was reached.
    DepthLimit,
    /// No de limit is available.
    NodeLimit,
    /// The configured definition limit was reached.
    DefinitionLimit,
    /// A schema reference names no available definition. Retains the `Arc<str>` detail.
    UnresolvedReference(Arc<str>),
    /// The schema reference name was bound to a different definition. Retains the `Arc<str>` detail.
    ConflictingReference(Arc<str>),
    /// The range failed validation.
    InvalidRange,
    /// The definition violates required construction rules.
    InvalidDefinition,
}

/// `Approximate` means the document may accept more than the matching codec,
/// never that the document may reject values that codec accepts.
pub struct JsonSchemaDocument {
    value: JsonValue,
    precision: RepresentationPrecision,
}

impl JsonSchemaDocument {
    #[must_use]
    /// Borrow the JSON value held by this representation.
    ///
    /// # Returns
    /// Type: `&JsonValue`.
    pub fn value(&self) -> &JsonValue {
        &self.value
    }

    #[must_use]
    /// Transfer the owned JSON value to the caller.
    ///
    /// # Returns
    /// Type: `JsonValue`.
    pub fn into_value(self) -> JsonValue {
        self.value
    }

    #[must_use]
    /// # Returns
    /// Type: `RepresentationPrecision`.
    pub const fn precision(&self) -> RepresentationPrecision {
        self.precision
    }
}

/// Typed exporter used by schema json schema.
struct Exporter {
    limits: RepresentationLimits,
    visited: usize,
    definitions: BTreeMap<Arc<str>, JsonValue>,
    targets: BTreeMap<Arc<str>, usize>,
    precision: RepresentationPrecision,
}

impl Exporter {
    ///
    /// # Parameters
    /// - `depth` (`usize`): Nesting depth used to enforce a parser or schema bound.
    ///
    /// # Returns
    /// Type: `Result<(), RepresentationError>`.
    fn visit(&mut self, depth: usize) -> Result<(), RepresentationError> {
        if depth > self.limits.depth.get() {
            return Err(RepresentationError::DepthLimit);
        }
        self.visited = self
            .visited
            .checked_add(1)
            .ok_or(RepresentationError::NodeLimit)?;
        if self.visited > self.limits.nodes.get() {
            return Err(RepresentationError::NodeLimit);
        }
        Ok(())
    }

    ///
    /// # Parameters
    /// - `depth` (`usize`): Nesting depth used to enforce a parser or schema bound.
    ///
    /// # Returns
    /// Type: `Result<usize, RepresentationError>`.
    fn child_depth(depth: usize) -> Result<usize, RepresentationError> {
        depth.checked_add(1).ok_or(RepresentationError::DepthLimit)
    }

    ///
    /// # Returns
    /// Type: `()`.
    fn approximate(&mut self) {
        self.precision = RepresentationPrecision::Approximate;
    }

    ///
    /// # Parameters
    /// - `schema` (`&SchemaAst`): Codec and validation rules for the application value.
    /// - `depth` (`usize`): Nesting depth used to enforce a parser or schema bound.
    ///
    /// # Returns
    /// Type: `Result<JsonValue, RepresentationError>`.
    fn shape(
        &mut self,
        schema: &SchemaAst,
        depth: usize,
    ) -> Result<JsonValue, RepresentationError> {
        self.visit(depth)?;
        let mut result = match schema.shape() {
            SchemaShape::Opaque => {
                self.approximate();
                Map::new()
            }
            SchemaShape::Primitive(primitive) => {
                let mut value = Map::new();
                let kind = match primitive {
                    PrimitiveShape::Any => None,
                    PrimitiveShape::Null => Some("null"),
                    PrimitiveShape::Boolean => Some("boolean"),
                    PrimitiveShape::String => Some("string"),
                    PrimitiveShape::Integer => {
                        // JSON Schema treats 1.0 as an integer, while the
                        // borrowed serde_json integer codecs use as_i64/u64.
                        self.approximate();
                        Some("integer")
                    }
                    PrimitiveShape::Number => Some("number"),
                    PrimitiveShape::Binary => {
                        self.approximate();
                        None
                    }
                };
                if let Some(kind) = kind {
                    value.insert("type".into(), JsonValue::String(kind.into()));
                }
                value
            }
            SchemaShape::Literal(literal) => {
                Map::from_iter([("const".into(), literal.as_ref().clone())])
            }
            SchemaShape::Sequence { element, maximum } => {
                let mut value = Map::from_iter([
                    ("type".into(), JsonValue::String("array".into())),
                    (
                        "items".into(),
                        self.shape(element, Self::child_depth(depth)?)?,
                    ),
                ]);
                if let Some(maximum) = maximum {
                    let count = u64::try_from(maximum.get())
                        .map_err(|_| RepresentationError::InvalidRange)?;
                    value.insert("maxItems".into(), JsonValue::Number(Number::from(count)));
                }
                value
            }
            SchemaShape::Tuple(items) => {
                let next = Self::child_depth(depth)?;
                let count =
                    u64::try_from(items.len()).map_err(|_| RepresentationError::InvalidRange)?;
                let mut prefix = Vec::with_capacity(items.len());
                for item in items {
                    prefix.push(self.shape(item, next)?);
                }
                Map::from_iter([
                    ("type".into(), JsonValue::String("array".into())),
                    ("prefixItems".into(), JsonValue::Array(prefix)),
                    ("minItems".into(), JsonValue::Number(Number::from(count))),
                    ("maxItems".into(), JsonValue::Number(Number::from(count))),
                ])
            }
            SchemaShape::Object {
                properties,
                unknown_fields,
            } => {
                let next = Self::child_depth(depth)?;
                let mut fields = Map::new();
                let mut required = Vec::new();
                let mut names = BTreeSet::new();
                for property in properties {
                    if !names.insert(property.name()) {
                        return Err(RepresentationError::InvalidDefinition);
                    }
                    let mut value = self.shape(property.schema(), next)?;
                    if let Some(default) = property.pure_default()
                        && let Some(object) = value.as_object_mut()
                    {
                        object.insert("default".into(), default.clone());
                    }
                    fields.insert(property.name().into(), value);
                    if property.required() {
                        required.push(JsonValue::String(property.name().into()));
                    }
                }
                let mut result = Map::from_iter([
                    ("type".into(), JsonValue::String("object".into())),
                    ("properties".into(), JsonValue::Object(fields)),
                    ("required".into(), JsonValue::Array(required)),
                ]);
                if *unknown_fields == UnknownFields::Reject {
                    result.insert("additionalProperties".into(), JsonValue::Bool(false));
                }
                result
            }
            SchemaShape::Union(variants) => {
                if variants.is_empty() {
                    return Err(RepresentationError::InvalidDefinition);
                }
                let next = Self::child_depth(depth)?;
                let mut branches = Vec::with_capacity(variants.len());
                for variant in variants {
                    branches.push(self.shape(variant, next)?);
                }
                Map::from_iter([("anyOf".into(), JsonValue::Array(branches))])
            }
            SchemaShape::Reference(reference) => {
                let name: Arc<str> = Arc::from(reference.name());
                let target = reference
                    .resolve()
                    .ok_or_else(|| RepresentationError::UnresolvedReference(Arc::clone(&name)))?;
                let identity = target.identity();
                if let Some(previous) = self.targets.get(&name) {
                    if *previous != identity {
                        return Err(RepresentationError::ConflictingReference(name));
                    }
                } else {
                    if self.definitions.len() >= self.limits.definitions.get() {
                        return Err(RepresentationError::DefinitionLimit);
                    }
                    self.targets.insert(Arc::clone(&name), identity);
                    self.definitions.insert(Arc::clone(&name), JsonValue::Null);
                    let definition = self.shape(&target, Self::child_depth(depth)?)?;
                    self.definitions.insert(Arc::clone(&name), definition);
                }
                let escaped = name.replace('~', "~0").replace('/', "~1");
                Map::from_iter([(
                    "$ref".into(),
                    JsonValue::String(format!("#/$defs/{escaped}")),
                )])
            }
            SchemaShape::Checked { source, .. } => {
                self.approximate();
                into_object(self.shape(source, Self::child_depth(depth)?)?)
            }
            SchemaShape::IntegerRange {
                source,
                minimum,
                maximum,
            } => {
                if minimum > maximum {
                    return Err(RepresentationError::InvalidRange);
                }
                let mut value = into_object(self.shape(source, Self::child_depth(depth)?)?);
                value.insert("minimum".into(), JsonValue::Number(Number::from(*minimum)));
                value.insert("maximum".into(), JsonValue::Number(Number::from(*maximum)));
                value
            }
            SchemaShape::UnsignedRange {
                source,
                minimum,
                maximum,
            } => {
                if minimum > maximum {
                    return Err(RepresentationError::InvalidRange);
                }
                let mut value = into_object(self.shape(source, Self::child_depth(depth)?)?);
                value.insert("minimum".into(), JsonValue::Number(Number::from(*minimum)));
                value.insert("maximum".into(), JsonValue::Number(Number::from(*maximum)));
                value
            }
            SchemaShape::StringLength {
                source,
                minimum,
                maximum,
                unit,
            } => {
                if minimum > maximum {
                    return Err(RepresentationError::InvalidRange);
                }
                let mut value = into_object(self.shape(source, Self::child_depth(depth)?)?);
                if *unit == StringLengthUnit::UnicodeScalars {
                    let minimum =
                        u64::try_from(*minimum).map_err(|_| RepresentationError::InvalidRange)?;
                    let maximum =
                        u64::try_from(*maximum).map_err(|_| RepresentationError::InvalidRange)?;
                    value.insert("minLength".into(), JsonValue::Number(Number::from(minimum)));
                    value.insert("maxLength".into(), JsonValue::Number(Number::from(maximum)));
                } else {
                    self.approximate();
                }
                value
            }
            SchemaShape::StringPattern { source, pattern } => {
                let mut value = into_object(self.shape(source, Self::child_depth(depth)?)?);
                let literal = match pattern {
                    StringPattern::Prefix(value)
                    | StringPattern::Contains(value)
                    | StringPattern::Suffix(value)
                    | StringPattern::Exact(value) => value,
                };
                if let Some(escaped) = ecma_literal(literal) {
                    let expression = match pattern {
                        StringPattern::Prefix(_) => format!("^{escaped}"),
                        StringPattern::Contains(_) => escaped,
                        StringPattern::Suffix(_) => {
                            self.approximate();
                            format!("{escaped}$")
                        }
                        StringPattern::Exact(_) => {
                            self.approximate();
                            format!("^{escaped}$")
                        }
                    };
                    value.insert("pattern".into(), JsonValue::String(expression));
                } else {
                    self.approximate();
                }
                value
            }
            SchemaShape::EncodedView(source) => {
                // A typed transform may reject an encoded value accepted by
                // its source codec, so the source document is only a safe
                // over-approximation of the transformed codec.
                self.approximate();
                into_object(self.shape(source, Self::child_depth(depth)?)?)
            }
        };
        for (name, value) in schema.annotations() {
            if name.as_ref() == "title" || name.as_ref() == "description" {
                result.insert(name.to_string(), JsonValue::String(value.to_string()));
            }
        }
        Ok(JsonValue::Object(result))
    }
}

/// Only printable ASCII literals have an unambiguous direct translation
/// between Rust substring matching and the supported ECMA regex subset.
///
/// # Parameters
/// - `value` (`&str`): Value read, transformed, encoded, or stored by this operation.
///
/// # Returns
/// Type: `Option<String>`.
/// Returns the value when present, or `None`.
fn ecma_literal(value: &str) -> Option<String> {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if !character.is_ascii_graphic() && character != ' ' {
            return None;
        }
        if matches!(
            character,
            '\\' | '^'
                | '$'
                | '.'
                | '*'
                | '+'
                | '?'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '|'
                | '/'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    Some(escaped)
}

///
/// # Parameters
/// - `value` (`JsonValue`): Value read, transformed, encoded, or stored by this operation.
///
/// # Returns
/// Type: `Map<String, JsonValue>`.
/// Returns `Map<String, JsonValue>`.
fn into_object(value: JsonValue) -> Map<String, JsonValue> {
    if let JsonValue::Object(object) = value {
        object
    } else {
        Map::new()
    }
}

/// Export a Draft 2020-12 document from the encoded-side AST. Unsupported
/// checks are omitted conservatively and mark the result approximate.
///
/// # Parameters
/// - `schema` (`&SchemaAst`): Codec and validation rules for the application value.
/// - `limits` (`RepresentationLimits`): Explicit resource bounds enforced while processing this input.
///
/// # Returns
/// Type: `Result<JsonSchemaDocument, RepresentationError>`.
pub fn export_json_schema(
    schema: &SchemaAst,
    limits: RepresentationLimits,
) -> Result<JsonSchemaDocument, RepresentationError> {
    let mut exporter = Exporter {
        limits,
        visited: 0,
        definitions: BTreeMap::new(),
        targets: BTreeMap::new(),
        precision: RepresentationPrecision::Exact,
    };
    let mut value = exporter.shape(schema, 1)?;
    if let JsonValue::Object(ref mut object) = value {
        object.insert(
            "$schema".into(),
            JsonValue::String("https://json-schema.org/draft/2020-12/schema".into()),
        );
        if !exporter.definitions.is_empty() {
            object.insert(
                "$defs".into(),
                JsonValue::Object(
                    exporter
                        .definitions
                        .into_iter()
                        .map(|(name, schema)| (name.to_string(), schema))
                        .collect(),
                ),
            );
        }
    }
    Ok(JsonSchemaDocument {
        value,
        precision: exporter.precision,
    })
}
