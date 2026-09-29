use super::internal::preparation::{PreparedStructure, prepare_structure};
use super::{JsonValue, ParseLimits, SchemaIssue, SchemaIssueKind, UnknownFields};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, Weak};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveShape {
    Any,
    Null,
    Boolean,
    String,
    Integer,
    Number,
    Binary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringLengthUnit {
    UnicodeScalars,
    Utf8Bytes,
    Graphemes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StringPattern {
    Prefix(Arc<str>),
    Contains(Arc<str>),
    Suffix(Arc<str>),
    Exact(Arc<str>),
}

impl StringPattern {
    #[must_use]
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
pub struct SchemaReference {
    name: Arc<str>,
    target: Weak<SchemaShape>,
}

impl SchemaReference {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// References are weak so a recursive definition releases its entire
    /// graph when its owner is dropped. A live schema retains the root shape.
    #[must_use]
    pub fn resolve(&self) -> Option<SchemaAst> {
        self.target.upgrade().map(|shape| SchemaAst {
            shape,
            annotations: Arc::new(BTreeMap::new()),
            prepared: Arc::new(Mutex::new(None)),
        })
    }
}

#[derive(Clone)]
pub struct SchemaProperty {
    name: Arc<str>,
    schema: SchemaAst,
    required: bool,
    pure_default: Option<Arc<JsonValue>>,
}

impl SchemaProperty {
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
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn schema(&self) -> &SchemaAst {
        &self.schema
    }

    #[must_use]
    pub const fn required(&self) -> bool {
        self.required
    }

    #[must_use]
    pub fn pure_default(&self) -> Option<&JsonValue> {
        self.pure_default.as_deref()
    }

    pub(crate) fn with_required(&self, required: bool) -> Self {
        Self {
            name: Arc::clone(&self.name),
            schema: self.schema.clone(),
            required,
            pure_default: self.pure_default.as_ref().map(Arc::clone),
        }
    }
}

/// Metadata describes encoded structure only. It carries no Rust type witness
/// and cannot itself construct or validate an application value.
pub enum SchemaShape {
    Opaque,
    Primitive(PrimitiveShape),
    Literal(Arc<JsonValue>),
    Sequence {
        element: SchemaAst,
        maximum: Option<NonZeroUsize>,
    },
    Tuple(Vec<SchemaAst>),
    Object {
        properties: Vec<SchemaProperty>,
        unknown_fields: UnknownFields,
    },
    Union(Vec<SchemaAst>),
    Reference(SchemaReference),
    Checked {
        source: SchemaAst,
        name: Arc<str>,
    },
    IntegerRange {
        source: SchemaAst,
        minimum: i64,
        maximum: i64,
    },
    UnsignedRange {
        source: SchemaAst,
        minimum: u64,
        maximum: u64,
    },
    StringLength {
        source: SchemaAst,
        minimum: usize,
        maximum: usize,
        unit: StringLengthUnit,
    },
    StringPattern {
        source: SchemaAst,
        pattern: StringPattern,
    },
    EncodedView(SchemaAst),
}

#[derive(Clone)]
pub struct SchemaAst {
    shape: Arc<SchemaShape>,
    annotations: Arc<BTreeMap<Arc<str>, Arc<str>>>,
    prepared: Arc<Mutex<Option<PreparedStructure>>>,
}

impl SchemaAst {
    #[must_use]
    pub fn opaque() -> Self {
        Self::new(SchemaShape::Opaque)
    }

    #[must_use]
    pub fn primitive(shape: PrimitiveShape) -> Self {
        Self::new(SchemaShape::Primitive(shape))
    }

    #[must_use]
    pub fn new(shape: SchemaShape) -> Self {
        Self {
            shape: Arc::new(shape),
            annotations: Arc::new(BTreeMap::new()),
            prepared: Arc::new(Mutex::new(None)),
        }
    }

    /// Reuse the source annotations when deriving a new structural view.
    #[must_use]
    pub fn derive(&self, shape: SchemaShape) -> Self {
        Self {
            shape: Arc::new(shape),
            annotations: Arc::clone(&self.annotations),
            prepared: Arc::new(Mutex::new(None)),
        }
    }

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
    pub fn shape(&self) -> &SchemaShape {
        &self.shape
    }

    pub(crate) fn shares_shape(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shape, &other.shape)
    }

    #[must_use]
    pub fn annotations(&self) -> &BTreeMap<Arc<str>, Arc<str>> {
        &self.annotations
    }

    #[must_use]
    pub(crate) fn identity(&self) -> usize {
        Arc::as_ptr(&self.shape) as usize
    }
}
