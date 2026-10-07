//! Structured schema issues and paths.

use std::sync::Arc;

/// A location in an encoded value. Property names belong to the schema or input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssueSegment {
    /// The issue is located at a named record field. Contains `Arc<str>`.
    Field(Arc<str>),
    /// The issue is located at a sequence index. Contains `usize`.
    Index(usize),
    /// The issue is located at a tagged enum variant. Contains `Arc<str>`.
    Variant(Arc<str>),
    /// A property-name constraint failed at this name, rather than in its encoded value.
    PropertyName(Arc<str>),
    /// Zero-based alternative index whose constraint or decode produced this issue.
    Alternative(usize),
}

/// Representation classes used in type mismatch diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// The encoded value is JSON null.
    Null,
    /// The encoded value is a boolean.
    Boolean,
    /// The encoded value is an integer.
    Integer,
    /// The encoded value is a JSON number.
    Number,
    /// The encoded value is a string.
    String,
    /// The encoded value is an ordered sequence.
    Sequence,
    /// The encoded value is a named-field record or JSON object.
    Record,
    /// The message contains binary data.
    Binary,
    /// The value belongs to a provider-defined representation class. Contains `Arc<str>`.
    Other(Arc<str>),
}

/// Distinct reasons for schema failure; callers need not inspect message text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaIssueKind {
    /// The type differs from the recorded or required value.
    TypeMismatch {
        /// The value required by the requested condition. Type: `ValueKind`.
        expected: ValueKind,
        /// The value observed when the requested condition failed. Type: `ValueKind`.
        actual: ValueKind,
    },
    /// The required field is missing. Carries `Arc<str>`.
    MissingField(Arc<str>),
    /// The field is not recognized. Carries `Arc<str>`.
    UnknownField(Arc<str>),
    /// The field already exists or occurred twice. Carries `Arc<str>`.
    DuplicateField(Arc<str>),
    /// The literal differs from the recorded or required value.
    LiteralMismatch,
    /// The pattern differs from the recorded or required value.
    PatternMismatch,
    /// The tag failed validation.
    InvalidTag,
    /// The recorded version differs from the declared version.
    VersionMismatch {
        /// The value required by the requested condition. Type: `u64`.
        expected: u64,
        /// The value observed when the requested condition failed. Type: `Option<u64>`.
        actual: Option<u64>,
    },
    /// The length differs from the recorded or required value.
    LengthMismatch {
        /// The value required by the requested condition. Type: `usize`.
        expected: usize,
        /// The value observed when the requested condition failed. Type: `usize`.
        actual: usize,
    },
    /// A numeric or ordered value lies outside the schema's accepted range.
    OutOfRange,
    /// A non-finite numeric value is not permitted by the schema.
    NonFinite,
    /// The value failed validation.
    InvalidValue {
        /// Stable code identifying the failed validation rule. Type: `Arc<str>`.
        code: Arc<str>,
    },
    /// The JSON input is syntactically invalid.
    JsonSyntax {
        /// Parser detail describing the syntax error. Type: `Arc<str>`.
        detail: Arc<str>,
    },
    /// A value could not be encoded as JSON.
    JsonEncoding {
        /// Encoder detail describing the failure. Type: `Arc<str>`.
        detail: Arc<str>,
    },
    /// The representation is not supported.
    UnsupportedRepresentation {
        /// Unsupported representation name. Type: `Arc<str>`.
        kind: Arc<str>,
    },
    /// Parsing or validation exceeded a configured resource limit.
    LimitExceeded {
        /// Resource whose limit was exceeded. Type: `SchemaResource`.
        resource: SchemaResource,
    },
    /// The definition violates required construction rules.
    InvalidDefinition,
    /// More than one schema issue was collected.
    Multiple,
    /// An owned input was already consumed.
    InputConsumed,
    /// An operation requires a genuine independent type-validation witness that the codec does not provide.
    MissingValidator,
    /// No branch matched; child issues retain bounded branch diagnostics.
    NoAlternative,
    /// More than one alternative matched where exactly one is required.
    AmbiguousAlternatives {
        /// Zero-based matching alternative indexes in declaration order.
        matches: Vec<usize>,
    },
    /// A named constraint failed with its declared operand or accepted range.
    Constraint {
        /// Constraint identifier, such as minimum or stringLength.
        name: Arc<str>,
        /// Constraint operand or range as diagnostic text, not an executable policy.
        operand: Arc<str>,
    },
    /// A configured operation would exceed a resource bound; this is not an ordinary branch mismatch.
    ResourceDemand {
        /// Resource whose demand exceeded its configured bound.
        resource: SchemaResource,
        /// Caller-supplied maximum for the selected resource.
        limit: usize,
        /// Required or observed demand, saturated when arithmetic cannot represent it.
        observed: usize,
    },
    /// A bounded pattern search exceeds its per-pattern demand or overflows its demand calculation.
    PatternBudget,
    /// Intersected encoding outputs disagree on a value that cannot be merged losslessly.
    MergeConflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The parser resource whose configured bound was exceeded.
pub enum SchemaResource {
    /// Encoded or input byte count, including JSON escaping and punctuation when writing.
    Bytes,
    /// Schema traversal depth, or legacy parser nesting depth where no separate JSON policy is supplied.
    Depth,
    /// Number of items or fields in one collection.
    Width,
    /// Visited schema or representation nodes, as charged by the operation.
    Steps,
    /// Issue-tree nodes retained during bounded accumulation, including composite nodes.
    Issues,
    /// JSON value nesting depth, counting the root as one independently of schema wrappers.
    JsonDepth,
    /// ASCII digits in a numeric token or materialized integer, as defined by the operation.
    IntegerDigits,
    /// Unicode scalar count in a string or property name.
    StringLength,
    /// Cumulative conservative pattern-search demand within a configured invocation.
    PatternSteps,
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
    /// # Parameters
    /// - `kind` (`SchemaIssueKind`): The kind supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new(kind: SchemaIssueKind) -> Self {
        Self {
            kind,
            path: Vec::new(),
            children: Vec::new(),
        }
    }

    #[must_use]
    /// Return the typed category of this value.
    ///
    /// # Returns
    /// Type: `&SchemaIssueKind`.
    pub fn kind(&self) -> &SchemaIssueKind {
        &self.kind
    }

    #[must_use]
    /// Borrow the ordered location segments of this schema issue.
    ///
    /// # Returns
    /// Type: `&[IssueSegment]`.
    pub fn path(&self) -> &[IssueSegment] {
        &self.path
    }

    #[must_use]
    /// Borrow nested schema issues collected under this parent issue.
    ///
    /// # Returns
    /// Type: `&[Self]`.
    pub fn children(&self) -> &[Self] {
        &self.children
    }

    #[must_use]
    /// Attach nested validation issues without losing the parent location.
    ///
    /// # Parameters
    /// - `children` (`Vec<Self>`): The children supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn with_children(mut self, children: Vec<Self>) -> Self {
        self.children = children;
        self
    }

    #[must_use]
    /// Add a field, index, or variant segment to this issue’s location.
    ///
    /// # Parameters
    /// - `segment` (`IssueSegment`): The segment supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
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
/// Whether decoding stops at the first issue or collects multiple issues.
pub enum SchemaErrorMode {
    /// Select the first matching candidate.
    First,
    /// Accumulate the new value with previously recorded state.
    Accumulate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The rule for object fields absent from a record schema.
pub enum UnknownFields {
    /// Ignore the unsupported or unknown input under this policy.
    Ignore,
    /// Reject the conflicting input without applying its proposed state change.
    Reject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The rule for repeated object fields during decoding.
pub enum DuplicateFields {
    /// Reject the conflicting input without applying its proposed state change.
    Reject,
    /// Keep the first value when duplicate input is encountered.
    KeepFirst,
    /// Replace the earlier value with the last duplicate.
    KeepLast,
}

/// All parser budgets are explicitly selected by the consumer/capability.
/// The library supplies no hidden depth, node, issue or allocation cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseLimits {
    /// The owned encoded bytes. Type: `std::num::NonZeroUsize`.
    pub bytes: std::num::NonZeroUsize,
    /// Maximum permitted nesting depth. Type: `std::num::NonZeroUsize`.
    pub depth: std::num::NonZeroUsize,
    /// Maximum permitted schema or parse nodes. Type: `std::num::NonZeroUsize`.
    pub nodes: std::num::NonZeroUsize,
    /// Maximum number of items in one collection. Type: `std::num::NonZeroUsize`.
    pub collection: std::num::NonZeroUsize,
    /// Maximum structured validation issues collected. Type: `std::num::NonZeroUsize`.
    pub issues: std::num::NonZeroUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Explicit size, depth, collection, and issue limits plus field-handling rules for parsing.
pub struct ParsePolicy {
    /// Explicit resource bounds applied to this operation. Type: `ParseLimits`.
    pub limits: ParseLimits,
    /// Policy for collecting or stopping at schema errors. Type: `SchemaErrorMode`.
    pub errors: SchemaErrorMode,
    /// Policy for input object fields absent from the schema. Type: `UnknownFields`.
    pub unknown_fields: UnknownFields,
    /// Policy for repeated input object field names. Type: `DuplicateFields`.
    pub duplicate_fields: DuplicateFields,
}

impl std::fmt::Display for SchemaIssue {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?} at {:?}", self.kind, self.path)
    }
}

impl std::error::Error for SchemaIssue {}
