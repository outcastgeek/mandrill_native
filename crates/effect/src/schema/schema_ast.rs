//! Immutable schema shapes and annotations.

use super::internal::preparation::{PreparedStructure, prepare_structure};
use super::{JsonValue, ParseLimits, SchemaIssue, SchemaIssueKind, UnknownFields};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, Weak};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The encoded representation expected by a primitive schema.
pub enum PrimitiveShape {
    /// Any matching candidate satisfies this condition.
    Any,
    /// The encoded value is JSON null.
    Null,
    /// The encoded value is a boolean.
    Boolean,
    /// The encoded value is a string.
    String,
    /// The encoded value is an integer.
    Integer,
    /// The encoded value is a JSON number.
    Number,
    /// The message contains binary data.
    Binary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The unit used to interpret a string length constraint.
pub enum StringLengthUnit {
    /// Measure string length in Unicode scalar values.
    UnicodeScalars,
    /// Measure string length by its UTF-8 byte count.
    Utf8Bytes,
    /// Measure string length in grapheme clusters.
    Graphemes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A compiled or declarative constraint applied to encoded strings.
pub enum StringPattern {
    /// Match the beginning of the supplied string. Contains `Arc<str>`.
    Prefix(Arc<str>),
    /// The selected value is present. Contains `Arc<str>`.
    Contains(Arc<str>),
    /// Match the end of the supplied string. Contains `Arc<str>`.
    Suffix(Arc<str>),
    /// The exported representation preserves the source constraint exactly. Contains `Arc<str>`.
    Exact(Arc<str>),
}

impl StringPattern {
    #[must_use]
    /// Check whether the encoded representation satisfies this schema shape.
    ///
    /// # Parameters
    /// - `value` (`&str`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub fn matches(&self, value: &str) -> bool {
        match self {
            Self::Prefix(pattern) => value.starts_with(pattern.as_ref()),
            Self::Contains(pattern) => value.contains(pattern.as_ref()),
            Self::Suffix(pattern) => value.ends_with(pattern.as_ref()),
            Self::Exact(pattern) => value == pattern.as_ref(),
        }
    }
}

#[derive(Clone)]
/// A named reference to a schema node used to represent recursive structure.
pub struct SchemaReference {
    name: Arc<str>,
    target: Weak<SchemaShape>,
}

impl SchemaReference {
    #[must_use]
    /// Borrow the stable name used to identify this definition.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// References are weak so a recursive definition releases its entire
    /// graph when its owner is dropped. A live schema retains the root shape.
    #[must_use]
    /// # Returns
    /// Type: `Option<SchemaAst>`.
    /// Returns the value when present, or `None`.
    pub fn resolve(&self) -> Option<SchemaAst> {
        self.target.upgrade().map(|shape| SchemaAst {
            shape,
            annotations: Arc::new(BTreeMap::new()),
            prepared: Arc::new(Mutex::new(None)),
        })
    }
}

#[derive(Clone)]
/// One named field and its representation constraints in a record schema.
pub struct SchemaProperty {
    name: Arc<str>,
    schema: SchemaAst,
    required: bool,
    pure_default: Option<Arc<JsonValue>>,
}

impl SchemaProperty {
    /// Construct a `SchemaProperty` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    /// - `schema` (`SchemaAst`): Codec and validation rules for the application value.
    /// - `required` (`bool`): The required supplied to this operation.
    /// - `pure_default` (`Option<Arc<JsonValue>>`): The pure default supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, SchemaIssue>`.
    pub fn new(
        name: impl Into<Arc<str>>,
        schema: SchemaAst,
        required: bool,
        pure_default: Option<Arc<JsonValue>>,
    ) -> Result<Self, SchemaIssue> {
        let name = name.into();
        if name.is_empty() {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        Ok(Self {
            name,
            schema,
            required,
            pure_default,
        })
    }

    #[must_use]
    /// Borrow the stable name used to identify this definition.
    ///
    /// # Returns
    /// Type: `&str`.
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    /// Borrow the schema associated with this builder or definition.
    ///
    /// # Returns
    /// Type: `&SchemaAst`.
    pub fn schema(&self) -> &SchemaAst {
        &self.schema
    }

    #[must_use]
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub const fn required(&self) -> bool {
        self.required
    }

    #[must_use]
    /// Construct a pure codec using the default representation policy.
    ///
    /// # Returns
    /// Type: `Option<&JsonValue>`.
    /// Returns the value when present, or `None`.
    pub fn pure_default(&self) -> Option<&JsonValue> {
        self.pure_default.as_deref()
    }
}

/// Metadata describes encoded structure only. It carries no Rust type witness
/// and cannot itself construct or validate an application value.
pub enum SchemaShape {
    /// The representation cannot be exported as a structural schema.
    Opaque,
    /// The schema represents one primitive encoded value. Contains `PrimitiveShape`.
    Primitive(PrimitiveShape),
    /// The encoded value must equal a declared literal. Contains `Arc<JsonValue>`.
    Literal(Arc<JsonValue>),
    /// The encoded value is an ordered sequence.
    Sequence {
        /// Schema applied to each sequence element. Type: `SchemaAst`.
        element: SchemaAst,
        /// Largest value permitted by this policy. Type: `Option<NonZeroUsize>`.
        maximum: Option<NonZeroUsize>,
    },
    /// The encoded value is a fixed-size ordered tuple. Contains `Vec<SchemaAst>`.
    Tuple(Vec<SchemaAst>),
    /// The encoded value is a record or JSON object.
    Object {
        /// Named properties accepted by this object schema. Type: `Vec<SchemaProperty>`.
        properties: Vec<SchemaProperty>,
        /// Policy for input object fields absent from the schema. Type: `UnknownFields`.
        unknown_fields: UnknownFields,
    },
    /// The schema accepts one of several declared alternatives. Contains `Vec<SchemaAst>`.
    Union(Vec<SchemaAst>),
    /// The schema node refers to another named definition. Contains `SchemaReference`.
    Reference(SchemaReference),
    /// The value passed its validation checks.
    Checked {
        /// The underlying failure or provider cause. Type: `SchemaAst`.
        source: SchemaAst,
        /// The stable name identifying this value. Type: `Arc<str>`.
        name: Arc<str>,
    },
    /// The integer lies outside its permitted range.
    IntegerRange {
        /// The underlying failure or provider cause. Type: `SchemaAst`.
        source: SchemaAst,
        /// Smallest value permitted by this policy. Type: `i64`.
        minimum: i64,
        /// Largest value permitted by this policy. Type: `i64`.
        maximum: i64,
    },
    /// The unsigned integer lies outside its permitted range.
    UnsignedRange {
        /// The underlying failure or provider cause. Type: `SchemaAst`.
        source: SchemaAst,
        /// Smallest value permitted by this policy. Type: `u64`.
        minimum: u64,
        /// Largest value permitted by this policy. Type: `u64`.
        maximum: u64,
    },
    /// The string violated its declared length bound.
    StringLength {
        /// The underlying failure or provider cause. Type: `SchemaAst`.
        source: SchemaAst,
        /// Smallest value permitted by this policy. Type: `usize`.
        minimum: usize,
        /// Largest value permitted by this policy. Type: `usize`.
        maximum: usize,
        /// Unit used to check this string length bound. Type: `StringLengthUnit`.
        unit: StringLengthUnit,
    },
    /// The string did not match its declared pattern.
    StringPattern {
        /// The underlying failure or provider cause. Type: `SchemaAst`.
        source: SchemaAst,
        /// Pattern constraint applied to encoded strings. Type: `StringPattern`.
        pattern: StringPattern,
    },
    /// The value is represented by already encoded data. Contains `SchemaAst`.
    EncodedView(SchemaAst),
}

#[derive(Clone)]
/// The representation tree used for validation and export, separate from constructing application values.
pub struct SchemaAst {
    shape: Arc<SchemaShape>,
    annotations: Arc<BTreeMap<Arc<str>, Arc<str>>>,
    prepared: Arc<Mutex<Option<PreparedStructure>>>,
}

impl SchemaAst {
    #[must_use]
    /// Describe a value whose structure cannot be exported as a schema AST.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn opaque() -> Self {
        Self::new(SchemaShape::Opaque)
    }

    #[must_use]
    /// Describe one primitive encoded representation and its constraints.
    ///
    /// # Parameters
    /// - `shape` (`PrimitiveShape`): Encoded-side structure required by this schema.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn primitive(shape: PrimitiveShape) -> Self {
        Self::new(SchemaShape::Primitive(shape))
    }

    #[must_use]
    /// Construct a `SchemaAst` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `shape` (`SchemaShape`): Encoded-side structure required by this schema.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(shape: SchemaShape) -> Self {
        Self {
            shape: Arc::new(shape),
            annotations: Arc::new(BTreeMap::new()),
            prepared: Arc::new(Mutex::new(None)),
        }
    }

    /// Reuse the source annotations when deriving a new structural view.
    #[must_use]
    /// # Parameters
    /// - `shape` (`SchemaShape`): Encoded-side structure required by this schema.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn derive(&self, shape: SchemaShape) -> Self {
        Self {
            shape: Arc::new(shape),
            annotations: Arc::clone(&self.annotations),
            prepared: Arc::new(Mutex::new(None)),
        }
    }

    /// Return a value with the supplied annotations added to its context.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    /// - `value` (`impl Into<Arc<str>>`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, SchemaIssue>`.
    pub fn annotate(
        &self,
        name: impl Into<Arc<str>>,
        value: impl Into<Arc<str>>,
    ) -> Result<Self, SchemaIssue> {
        let name = name.into();
        if name.is_empty() {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        let mut annotations = self.annotations.as_ref().clone();
        annotations.insert(name, value.into());
        Ok(Self {
            shape: Arc::clone(&self.shape),
            annotations: Arc::new(annotations),
            prepared: Arc::clone(&self.prepared),
        })
    }

    /// Describe a record representation from its named fields.
    ///
    /// # Parameters
    /// - `fields` (`Vec<SchemaProperty>`): Typed fields composing this record or schema.
    /// - `unknown_fields` (`UnknownFields`): The unknown fields supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, SchemaIssue>`.
    pub fn object(
        fields: Vec<SchemaProperty>,
        unknown_fields: UnknownFields,
    ) -> Result<Self, SchemaIssue> {
        let mut names = std::collections::BTreeSet::new();
        for field in &fields {
            if !names.insert(Arc::clone(&field.name)) {
                return Err(SchemaIssue::new(SchemaIssueKind::DuplicateField(
                    Arc::clone(&field.name),
                )));
            }
        }
        Ok(Self::new(SchemaShape::Object {
            properties: fields,
            unknown_fields,
        }))
    }

    /// Describe a recursive reference without strongly retaining the schema graph.
    ///
    /// # Parameters
    /// - `name` (`impl Into<Arc<str>>`): Stable name used to register or locate this definition.
    /// - `build` (`impl FnOnce(SchemaReference) -> SchemaShape`): The build supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<Self, SchemaIssue>`.
    pub fn recursive(
        name: impl Into<Arc<str>>,
        build: impl FnOnce(SchemaReference) -> SchemaShape,
    ) -> Result<Self, SchemaIssue> {
        let name = name.into();
        if name.is_empty() {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        let shape = Arc::new_cyclic(|target| {
            build(SchemaReference {
                name,
                target: target.clone(),
            })
        });
        Ok(Self {
            shape,
            annotations: Arc::new(BTreeMap::new()),
            prepared: Arc::new(Mutex::new(None)),
        })
    }

    /// Validate the encoded structure on first use, with caller-selected
    /// budgets. A failed small-budget scan is not cached, so a later caller
    /// may choose a larger budget. Successful structure analysis is shared
    /// by every clone, including across concurrent first callers.
    ///
    /// # Parameters
    /// - `limits` (`ParseLimits`): Explicit resource bounds enforced while processing this input.
    ///
    /// # Returns
    /// Type: `Result<(), SchemaIssue>`.
    pub fn prepare(&self, limits: ParseLimits) -> Result<(), SchemaIssue> {
        let mut cached = self
            .prepared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let structure = match *cached {
            Some(structure) => structure,
            None => {
                let structure = prepare_structure(self, limits)?;
                *cached = Some(structure);
                structure
            }
        };
        structure.within(limits)
    }

    #[must_use]
    /// Borrow the encoded-side representation shape.
    ///
    /// # Returns
    /// Type: `&SchemaShape`.
    pub fn shape(&self) -> &SchemaShape {
        &self.shape
    }

    /// Borrow tagged-union discriminator annotation when present; this metadata does not rewrite wire tags.
    #[must_use]
    pub fn discriminant(&self) -> Option<&str> {
        self.annotations.get("discriminant").map(AsRef::as_ref)
    }

    #[must_use]
    /// Borrow the metadata attached to this schema node.
    ///
    /// # Returns
    /// Type: `&BTreeMap<Arc<str>, Arc<str>>`.
    pub fn annotations(&self) -> &BTreeMap<Arc<str>, Arc<str>> {
        &self.annotations
    }

    #[must_use]
    /// # Returns
    /// Type: `usize`.
    pub(crate) fn identity(&self) -> usize {
        Arc::as_ptr(&self.shape) as usize
    }
}
