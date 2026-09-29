use std::sync::Arc;

/// A location in an encoded value. Property names belong to the schema or input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssueSegment {
    Field(Arc<str>),
    Index(usize),
    Variant(Arc<str>),
}

/// Representation classes used in type mismatch diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Null,
    Boolean,
    Integer,
    Number,
    String,
    Sequence,
    Record,
    Binary,
    Other(Arc<str>),
}

/// Distinct reasons for schema failure; callers need not inspect message text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaIssueKind {
    TypeMismatch {
        expected: ValueKind,
        actual: ValueKind,
    },
    MissingField(Arc<str>),
    UnknownField(Arc<str>),
    DuplicateField(Arc<str>),
    LiteralMismatch,
    PatternMismatch,
    InvalidTag,
    VersionMismatch {
        expected: u64,
        actual: Option<u64>,
    },
    LengthMismatch {
        expected: usize,
        actual: usize,
    },
    OutOfRange,
    NonFinite,
    InvalidValue {
        code: Arc<str>,
    },
    JsonSyntax {
        detail: Arc<str>,
    },
    JsonEncoding {
        detail: Arc<str>,
    },
    UnsupportedRepresentation {
        kind: Arc<str>,
    },
    LimitExceeded {
        resource: SchemaResource,
    },
    InvalidDefinition,
    Multiple,
    InputConsumed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaResource {
    Bytes,
    Depth,
    Width,
    Steps,
    Issues,
}

/// A typed issue with its encoded-side path, ordered from root to leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaIssue {
    kind: SchemaIssueKind,
    path: Vec<IssueSegment>,
    children: Vec<Self>,
}

impl SchemaIssue {
    #[must_use]
    pub const fn new(kind: SchemaIssueKind) -> Self {
        Self {
            kind,
            path: Vec::new(),
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn kind(&self) -> &SchemaIssueKind {
        &self.kind
    }

    #[must_use]
    pub fn path(&self) -> &[IssueSegment] {
        &self.path
    }

    #[must_use]
    pub fn children(&self) -> &[Self] {
        &self.children
    }

    #[must_use]
    pub fn with_children(mut self, children: Vec<Self>) -> Self {
        self.children = children;
        self
    }

    #[must_use]
    pub fn at(mut self, segment: IssueSegment) -> Self {
        self.path.insert(0, segment.clone());
        self.children = self
            .children
            .into_iter()
            .map(|issue| issue.at(segment.clone()))
            .collect();
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaErrorMode {
    First,
    Accumulate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnknownFields {
    Ignore,
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuplicateFields {
    Reject,
    KeepFirst,
    KeepLast,
}

/// All parser budgets are explicitly selected by the consumer/capability.
/// The library supplies no hidden depth, node, issue or allocation cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseLimits {
    pub bytes: std::num::NonZeroUsize,
    pub depth: std::num::NonZeroUsize,
    pub nodes: std::num::NonZeroUsize,
    pub collection: std::num::NonZeroUsize,
    pub issues: std::num::NonZeroUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsePolicy {
    pub limits: ParseLimits,
    pub errors: SchemaErrorMode,
    pub unknown_fields: UnknownFields,
    pub duplicate_fields: DuplicateFields,
}

impl std::fmt::Display for SchemaIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?} at {:?}", self.kind, self.path)
    }
}

impl std::error::Error for SchemaIssue {}
