//! Structured schema issues and paths.

use std::sync::Arc;

/// A location in an encoded value. Property names belong to the schema or input.
pub enum IssueSegment {
    /// The issue is located at a named record field. Contains `Arc<str>`.
    Field(Arc<str>),
    /// A field name retained in prepared text backing.
    PreparedField(PreparedDiagnosticText),
    /// The issue is located at a sequence index. Contains `usize`.
    Index(usize),
    /// The issue is located at a tagged enum variant. Contains `Arc<str>`.
    Variant(Arc<str>),
    /// A property-name constraint failed at this name, rather than in its encoded value.
    PropertyName(Arc<str>),
    /// A property name retained in prepared text backing.
    PreparedPropertyName(PreparedDiagnosticText),
    /// Zero-based alternative index whose constraint or decode produced this issue.
    Alternative(usize),
}

#[derive(Debug, PartialEq, Eq)]
enum SegmentView<'a> {
    Field(&'a str),
    Index(usize),
    Variant(&'a str),
    PropertyName(&'a str),
    Alternative(usize),
}
impl IssueSegment {
    fn view(&self) -> SegmentView<'_> {
        match self {
            Self::Field(v) => SegmentView::Field(v),
            Self::PreparedField(v) => SegmentView::Field(v.as_str()),
            Self::Index(v) => SegmentView::Index(*v),
            Self::Variant(v) => SegmentView::Variant(v),
            Self::PropertyName(v) => SegmentView::PropertyName(v),
            Self::PreparedPropertyName(v) => SegmentView::PropertyName(v.as_str()),
            Self::Alternative(v) => SegmentView::Alternative(*v),
        }
    }
    /// Borrow a field, property or variant name regardless of its storage.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self.view() {
            SegmentView::Field(v) | SegmentView::Variant(v) | SegmentView::PropertyName(v) => {
                Some(v)
            }
            SegmentView::Index(_) | SegmentView::Alternative(_) => None,
        }
    }
    /// Retain prepared text without allocating a native string.
    pub fn try_clone_prepared(&self) -> Result<Self, crate::memory::MemoryError> {
        Ok(match self {
            Self::PreparedField(v) => Self::PreparedField(v.try_clone()?),
            Self::PreparedPropertyName(v) => Self::PreparedPropertyName(v.try_clone()?),
            Self::Field(v) => Self::Field(Arc::clone(v)),
            Self::PropertyName(v) => Self::PropertyName(Arc::clone(v)),
            Self::Variant(v) => Self::Variant(Arc::clone(v)),
            Self::Index(v) => Self::Index(*v),
            Self::Alternative(v) => Self::Alternative(*v),
        })
    }
}
impl Clone for IssueSegment {
    fn clone(&self) -> Self {
        match self {
            Self::Field(v) => Self::Field(Arc::clone(v)),
            Self::PreparedField(v) => Self::Field(Arc::from(v.as_str())),
            Self::PropertyName(v) => Self::PropertyName(Arc::clone(v)),
            Self::PreparedPropertyName(v) => Self::PropertyName(Arc::from(v.as_str())),
            Self::Variant(v) => Self::Variant(Arc::clone(v)),
            Self::Index(v) => Self::Index(*v),
            Self::Alternative(v) => Self::Alternative(*v),
        }
    }
}
impl std::fmt::Debug for IssueSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.view().fmt(f)
    }
}
impl PartialEq for IssueSegment {
    fn eq(&self, other: &Self) -> bool {
        self.view() == other.view()
    }
}
impl Eq for IssueSegment {}

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
pub enum SchemaIssueKind {
    /// Physical prepared-memory ownership or admission failed.
    Memory(crate::memory::MemoryError),
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
    /// Unknown field name retained in prepared text backing.
    PreparedUnknownField(PreparedDiagnosticText),
    /// The field already exists or occurred twice. Carries `Arc<str>`.
    DuplicateField(Arc<str>),
    /// Duplicate field name retained in prepared text backing.
    PreparedDuplicateField(PreparedDiagnosticText),
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
    /// JSON syntax detail retained in prepared text backing.
    PreparedJsonSyntax { detail: PreparedDiagnosticText },
    /// A value could not be encoded as JSON.
    JsonEncoding {
        /// Encoder detail describing the failure. Type: `Arc<str>`.
        detail: Arc<str>,
    },
    /// JSON encoding detail retained in prepared text backing.
    PreparedJsonEncoding { detail: PreparedDiagnosticText },
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
    /// The prepared codec has no genuine direct-writer witness.
    MissingWriter,
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

#[derive(Debug, PartialEq, Eq)]
enum KindView<'a> {
    Memory(&'a crate::memory::MemoryError),
    MissingField(&'a Arc<str>),
    UnknownField(&'a str),
    DuplicateField(&'a str),
    LiteralMismatch,
    PatternMismatch,
    InvalidTag,
    OutOfRange,
    NonFinite,
    InvalidDefinition,
    Multiple,
    InputConsumed,
    MissingValidator,
    MissingWriter,
    NoAlternative,
    PatternBudget,
    MergeConflict,
    TypeMismatch {
        expected: &'a ValueKind,
        actual: &'a ValueKind,
    },
    VersionMismatch {
        expected: &'a u64,
        actual: &'a Option<u64>,
    },
    LengthMismatch {
        expected: &'a usize,
        actual: &'a usize,
    },
    InvalidValue {
        code: &'a Arc<str>,
    },
    JsonSyntax {
        detail: &'a str,
    },
    JsonEncoding {
        detail: &'a str,
    },
    UnsupportedRepresentation {
        kind: &'a Arc<str>,
    },
    LimitExceeded {
        resource: &'a SchemaResource,
    },
    AmbiguousAlternatives {
        matches: &'a Vec<usize>,
    },
    Constraint {
        name: &'a Arc<str>,
        operand: &'a Arc<str>,
    },
    ResourceDemand {
        resource: &'a SchemaResource,
        limit: &'a usize,
        observed: &'a usize,
    },
}
impl SchemaIssueKind {
    fn view(&self) -> KindView<'_> {
        match self {
            Self::Memory(v) => KindView::Memory(v),
            Self::MissingField(v) => KindView::MissingField(v),
            Self::UnknownField(v) => KindView::UnknownField(v),
            Self::PreparedUnknownField(v) => KindView::UnknownField(v.as_str()),
            Self::DuplicateField(v) => KindView::DuplicateField(v),
            Self::PreparedDuplicateField(v) => KindView::DuplicateField(v.as_str()),
            Self::LiteralMismatch => KindView::LiteralMismatch,
            Self::PatternMismatch => KindView::PatternMismatch,
            Self::InvalidTag => KindView::InvalidTag,
            Self::OutOfRange => KindView::OutOfRange,
            Self::NonFinite => KindView::NonFinite,
            Self::InvalidDefinition => KindView::InvalidDefinition,
            Self::Multiple => KindView::Multiple,
            Self::InputConsumed => KindView::InputConsumed,
            Self::MissingValidator => KindView::MissingValidator,
            Self::MissingWriter => KindView::MissingWriter,
            Self::NoAlternative => KindView::NoAlternative,
            Self::PatternBudget => KindView::PatternBudget,
            Self::MergeConflict => KindView::MergeConflict,
            Self::TypeMismatch { expected, actual } => KindView::TypeMismatch { expected, actual },
            Self::VersionMismatch { expected, actual } => {
                KindView::VersionMismatch { expected, actual }
            }
            Self::LengthMismatch { expected, actual } => {
                KindView::LengthMismatch { expected, actual }
            }
            Self::InvalidValue { code } => KindView::InvalidValue { code },
            Self::JsonSyntax { detail } => KindView::JsonSyntax { detail },
            Self::PreparedJsonSyntax { detail } => KindView::JsonSyntax {
                detail: detail.as_str(),
            },
            Self::JsonEncoding { detail } => KindView::JsonEncoding { detail },
            Self::PreparedJsonEncoding { detail } => KindView::JsonEncoding {
                detail: detail.as_str(),
            },
            Self::UnsupportedRepresentation { kind } => {
                KindView::UnsupportedRepresentation { kind }
            }
            Self::LimitExceeded { resource } => KindView::LimitExceeded { resource },
            Self::AmbiguousAlternatives { matches } => KindView::AmbiguousAlternatives { matches },
            Self::Constraint { name, operand } => KindView::Constraint { name, operand },
            Self::ResourceDemand {
                resource,
                limit,
                observed,
            } => KindView::ResourceDemand {
                resource,
                limit,
                observed,
            },
        }
    }
    /// Borrow the dynamic diagnostic text for either native or prepared storage.
    #[must_use]
    pub fn diagnostic_text(&self) -> Option<&str> {
        if let KindView::UnknownField(v) | KindView::DuplicateField(v) = self.view() {
            Some(v)
        } else if let KindView::JsonSyntax { detail } | KindView::JsonEncoding { detail } =
            self.view()
        {
            Some(detail)
        } else {
            None
        }
    }
    /// Retain actual prepared text owners without materializing native text.
    /// The caller handles every nonempty AmbiguousAlternatives list with its
    /// preadmitted vector and return lease before reaching this helper.
    fn try_clone_prepared_text(&self) -> Result<Self, crate::memory::MemoryError> {
        match self {
            Self::PreparedUnknownField(v) => Ok(Self::PreparedUnknownField(v.try_clone()?)),
            Self::PreparedDuplicateField(v) => Ok(Self::PreparedDuplicateField(v.try_clone()?)),
            Self::PreparedJsonSyntax { detail } => Ok(Self::PreparedJsonSyntax {
                detail: detail.try_clone()?,
            }),
            Self::PreparedJsonEncoding { detail } => Ok(Self::PreparedJsonEncoding {
                detail: detail.try_clone()?,
            }),
            Self::Memory(_)
            | Self::TypeMismatch { .. }
            | Self::MissingField(_)
            | Self::UnknownField(_)
            | Self::DuplicateField(_)
            | Self::LiteralMismatch
            | Self::PatternMismatch
            | Self::InvalidTag
            | Self::VersionMismatch { .. }
            | Self::LengthMismatch { .. }
            | Self::OutOfRange
            | Self::NonFinite
            | Self::InvalidValue { .. }
            | Self::JsonSyntax { .. }
            | Self::JsonEncoding { .. }
            | Self::UnsupportedRepresentation { .. }
            | Self::LimitExceeded { .. }
            | Self::InvalidDefinition
            | Self::Multiple
            | Self::InputConsumed
            | Self::MissingValidator
            | Self::MissingWriter
            | Self::NoAlternative
            | Self::AmbiguousAlternatives { .. }
            | Self::Constraint { .. }
            | Self::ResourceDemand { .. }
            | Self::PatternBudget
            | Self::MergeConflict => Ok(self.clone()),
        }
    }
}
impl Clone for SchemaIssueKind {
    fn clone(&self) -> Self {
        match self {
            Self::Memory(v) => Self::Memory(*v),
            Self::MissingField(v) => Self::MissingField(v.clone()),
            Self::UnknownField(v) => Self::UnknownField(v.clone()),
            Self::DuplicateField(v) => Self::DuplicateField(v.clone()),
            Self::PreparedUnknownField(v) => Self::UnknownField(Arc::from(v.as_str())),
            Self::PreparedDuplicateField(v) => Self::DuplicateField(Arc::from(v.as_str())),
            Self::LiteralMismatch => Self::LiteralMismatch,
            Self::PatternMismatch => Self::PatternMismatch,
            Self::InvalidTag => Self::InvalidTag,
            Self::OutOfRange => Self::OutOfRange,
            Self::NonFinite => Self::NonFinite,
            Self::InvalidDefinition => Self::InvalidDefinition,
            Self::Multiple => Self::Multiple,
            Self::InputConsumed => Self::InputConsumed,
            Self::MissingValidator => Self::MissingValidator,
            Self::MissingWriter => Self::MissingWriter,
            Self::NoAlternative => Self::NoAlternative,
            Self::PatternBudget => Self::PatternBudget,
            Self::MergeConflict => Self::MergeConflict,
            Self::TypeMismatch { expected, actual } => Self::TypeMismatch {
                expected: expected.clone(),
                actual: actual.clone(),
            },
            Self::VersionMismatch { expected, actual } => Self::VersionMismatch {
                expected: *expected,
                actual: *actual,
            },
            Self::LengthMismatch { expected, actual } => Self::LengthMismatch {
                expected: *expected,
                actual: *actual,
            },
            Self::InvalidValue { code } => Self::InvalidValue { code: code.clone() },
            Self::JsonSyntax { detail } => Self::JsonSyntax {
                detail: detail.clone(),
            },
            Self::PreparedJsonSyntax { detail } => Self::JsonSyntax {
                detail: Arc::from(detail.as_str()),
            },
            Self::JsonEncoding { detail } => Self::JsonEncoding {
                detail: detail.clone(),
            },
            Self::PreparedJsonEncoding { detail } => Self::JsonEncoding {
                detail: Arc::from(detail.as_str()),
            },
            Self::UnsupportedRepresentation { kind } => {
                Self::UnsupportedRepresentation { kind: kind.clone() }
            }
            Self::LimitExceeded { resource } => Self::LimitExceeded {
                resource: *resource,
            },
            Self::AmbiguousAlternatives { matches } => Self::AmbiguousAlternatives {
                matches: matches.clone(),
            },
            Self::Constraint { name, operand } => Self::Constraint {
                name: name.clone(),
                operand: operand.clone(),
            },
            Self::ResourceDemand {
                resource,
                limit,
                observed,
            } => Self::ResourceDemand {
                resource: *resource,
                limit: *limit,
                observed: *observed,
            },
        }
    }
}
impl std::fmt::Debug for SchemaIssueKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.view().fmt(f)
    }
}
impl PartialEq for SchemaIssueKind {
    fn eq(&self, other: &Self) -> bool {
        self.view() == other.view()
    }
}
impl Eq for SchemaIssueKind {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The parser resource whose configured bound was exceeded.
pub enum SchemaResource {
    /// Encoded or input byte count, including JSON escaping and punctuation when writing.
    Bytes,
    /// Schema traversal depth, or legacy parser nesting depth where no separate JSON policy is supplied.
    Depth,
    /// Number of items or fields in one collection.
    Width,
    /// Concurrent prepared JSON document records.
    JsonDocuments,
    /// Concurrent prepared issue-tree traversal records.
    IssueTraversals,
    /// Concurrent prepared JSON representation check records.
    RepresentationChecks,
    /// Concurrent prepared schema context path records.
    ContextPaths,
    /// Concurrent prepared JSON codec boundary snapshot slots.
    WriterBoundaries,
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

enum IssueChildren {
    Native(Vec<SchemaIssue>),
    Prepared(super::SharedPreparedJsonArray<SchemaIssue>),
}
impl IssueChildren {
    fn as_slice(&self) -> &[SchemaIssue] {
        match self {
            Self::Native(value) => value,
            Self::Prepared(value) => value.as_slice(),
        }
    }
}
// Cloning is the existing ordinary owned-copy operation, even for prepared input.
impl Clone for IssueChildren {
    fn clone(&self) -> Self {
        Self::Native(self.as_slice().to_vec())
    }
}
impl std::fmt::Debug for IssueChildren {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}
impl PartialEq for IssueChildren {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}
impl Eq for IssueChildren {}

enum IssuePath {
    Native(Vec<IssueSegment>),
    Prepared(super::SharedPreparedJsonArray<IssueSegment>),
}
impl IssuePath {
    fn as_slice(&self) -> &[IssueSegment] {
        match self {
            Self::Native(path) => path,
            Self::Prepared(path) => path.as_slice(),
        }
    }
}
impl Clone for IssuePath {
    fn clone(&self) -> Self {
        Self::Native(self.as_slice().to_vec())
    }
}
impl std::fmt::Debug for IssuePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_slice().fmt(f)
    }
}
impl PartialEq for IssuePath {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}
impl Eq for IssuePath {}

/// Explicit physical path and child-array capacities for prepared diagnostics.
#[derive(Clone, Copy, Debug)]
pub struct PreparedIssueMemoryCapacity {
    /// Path arrays and segments per path.
    pub paths: super::PreparedJsonArrayCapacity,
    /// Child arrays and issues per array.
    pub children: super::PreparedJsonArrayCapacity,
    /// Concurrent text owners and UTF-8 bytes per owner.
    pub text: super::PreparedJsonArrayCapacity,
    /// Concurrent nonempty ambiguous-alternative lists.
    pub match_lists: usize,
    /// Minimum native vector capacity per match list.
    pub matches_per_list: usize,
    /// Concurrent issues retaining prepared path, child or match-list owners.
    pub records: usize,
}

/// Actual region and separately supplied frame demand for each diagnostic array kind.
#[derive(Clone, Copy, Debug)]
pub struct PreparedIssueMemoryDemand {
    /// Path vectors and their typed retained frames.
    pub paths: super::PreparedJsonArrayDemand,
    /// Child vectors and their typed retained frames.
    pub children: super::PreparedJsonArrayDemand,
    /// Text vectors and their typed retained frames.
    pub text: super::PreparedJsonArrayDemand,
    /// Combined region bytes, excluding frame backing and allocator overhead.
    pub region_bytes: crate::memory::Bytes,
    /// Requested native vector payload bytes; actual capacity is reported after preparation.
    pub requested_match_bytes: crate::memory::Bytes,
}

impl PreparedIssueMemoryCapacity {
    /// Calculate actual array storage and separately supplied frame layouts.
    pub fn demand(self) -> Result<PreparedIssueMemoryDemand, crate::memory::MemoryError> {
        let paths = self.paths.demand::<IssueSegment>()?;
        let children = self.children.demand::<SchemaIssue>()?;
        let text = self.text.demand::<u8>()?;
        let match_slots =
            crate::memory::SlotPool::<Option<Vec<usize>>>::storage_bytes(self.match_lists)?;
        let records = crate::memory::SlotPool::<IssueBacking>::storage_bytes(self.records)?;
        let requested_match_bytes = crate::memory::Bytes::new(
            std::alloc::Layout::array::<usize>(self.matches_per_list)
                .map_err(|_| crate::memory::MemoryError::InvalidLayout)?
                .size(),
        )
        .checked_mul(self.match_lists)?;
        Ok(PreparedIssueMemoryDemand {
            region_bytes: paths
                .region_bytes
                .checked_add(children.region_bytes)?
                .checked_add(text.region_bytes)?
                .checked_add(match_slots)?
                .checked_add(records)?,
            paths,
            children,
            text,
            requested_match_bytes,
        })
    }
}

/// Opt-in physical backing for prepared issue paths and child trees.
pub struct PreparedIssueMemory {
    paths: super::PreparedJsonArrayPool<IssueSegment>,
    children: super::PreparedJsonArrayPool<SchemaIssue>,
    text: super::PreparedJsonArrayPool<u8>,
    matches: crate::memory::SlotPool<'static, Option<Vec<usize>>>,
    records: crate::memory::SlotPool<'static, IssueBacking>,
    actual_match_bytes: crate::memory::Bytes,
}

impl PreparedIssueMemory {
    pub(super) fn child_pool(&self) -> &super::PreparedJsonArrayPool<SchemaIssue> {
        &self.children
    }
    pub(super) fn reserve_match_indices(
        &self,
        minimum: usize,
    ) -> Result<PreparedMatchIndices, crate::memory::MemoryError> {
        use crate::memory::MemoryError;
        let owner = IssueOwner(self.records.checkout()?);
        let mut lease = self.matches.checkout()?;
        let values = lease.take().ok_or(MemoryError::NoSlot)?;
        if values.capacity() < minimum {
            *lease = Some(values);
            return Err(MemoryError::NoSlot);
        }
        Ok(PreparedMatchIndices {
            owner: Some(owner),
            lease: Some(lease),
            values: Some(values),
        })
    }
    /// Prepare all pools using the supplied, independently sized frame pools.
    pub fn prepare(
        memory: &crate::memory::PreparedMemory<'static>,
        region: crate::memory::RegionId,
        capacity: PreparedIssueMemoryCapacity,
        path_frames: crate::memory::FramePool<'static>,
        child_frames: crate::memory::FramePool<'static>,
        text_frames: crate::memory::FramePool<'static>,
    ) -> Result<Self, crate::memory::MemoryError> {
        let _ = capacity.demand()?;
        let paths =
            super::PreparedJsonArrayPool::prepare(memory, region, capacity.paths, path_frames)?;
        let children =
            super::PreparedJsonArrayPool::prepare(memory, region, capacity.children, child_frames)?;
        let text =
            super::PreparedJsonArrayPool::prepare(memory, region, capacity.text, text_frames)?;
        let records = memory.slots(region, capacity.records, |_| Ok(IssueBacking::empty()))?;
        let mut actual_match_bytes = crate::memory::Bytes::new(0);
        let matches = memory.slots(region, capacity.match_lists, |_| {
            let mut values = Vec::<usize>::new();
            values
                .try_reserve_exact(capacity.matches_per_list)
                .map_err(|_| crate::memory::MemoryError::BackingAllocationFailed)?;
            actual_match_bytes = actual_match_bytes.checked_add(
                crate::memory::Bytes::new(values.capacity())
                    .checked_mul(std::mem::size_of::<usize>())?,
            )?;
            Ok(Some(values))
        })?;
        Ok(Self {
            paths,
            children,
            text,
            matches,
            records,
            actual_match_bytes,
        })
    }
    /// Sum of actual startup vector capacities in bytes, excluding allocator overhead.
    #[must_use]
    pub fn actual_match_bytes(&self) -> crate::memory::Bytes {
        self.actual_match_bytes
    }

    /// Admit actual text bytes and their retained frame before formatting.
    pub fn reserve_text(
        &self,
    ) -> Result<PreparedDiagnosticTextBuilder, crate::memory::MemoryError> {
        Ok(PreparedDiagnosticTextBuilder {
            bytes: self.text.reserve_shared()?,
            failure: None,
        })
    }

    pub(super) fn locate(
        &self,
        issue: SchemaIssue,
        path: impl IntoIterator<Item = Result<IssueSegment, SchemaIssue>>,
    ) -> SchemaIssue {
        let result = (|| {
            let mut segments = None;
            for segment in path {
                let segment = segment?;
                if segments.is_none() {
                    segments = Some(
                        self.paths
                            .reserve_shared()
                            .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?,
                    );
                }
                let Some(segments) = &mut segments else {
                    return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
                };
                segments.try_push(segment).map_err(|_| {
                    SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::NoSlot))
                })?;
            }
            Ok::<_, SchemaIssue>(segments.map(super::PreparedJsonArrayBuilder::publish))
        })();
        match result {
            Ok(Some(path)) => issue.with_prepared_path(path),
            Ok(None) => issue,
            Err(error) if super::prepared_json_validation::physical_cause(&issue).is_none() => {
                error
            }
            Err(_) => issue,
        }
    }
}

type MatchLease = crate::memory::SlotLease<'static, Option<Vec<usize>>>;
pub(super) struct PreparedMatchIndices {
    owner: Option<IssueOwner>,
    lease: Option<MatchLease>,
    values: Option<Vec<usize>>,
}
impl PreparedMatchIndices {
    pub fn as_slice(&self) -> &[usize] {
        self.values.as_deref().unwrap_or(&[])
    }
    pub fn push(&mut self, value: usize) -> Result<(), crate::memory::MemoryError> {
        let values = self
            .values
            .as_mut()
            .ok_or(crate::memory::MemoryError::NoSlot)?;
        if values.len() == values.capacity() {
            return Err(crate::memory::MemoryError::NoSlot);
        }
        values.push(value);
        Ok(())
    }
    #[allow(clippy::expect_used)] // The admitted guard owns all three parts until this final transfer.
    pub fn into_issue(mut self) -> SchemaIssue {
        let values = self.values.take().expect("admitted match indices");
        let lease = self.lease.take().expect("admitted match lease");
        let mut owner = self.owner.take().expect("admitted match record");
        owner.0.match_lease = Some(lease);
        SchemaIssue {
            kind: SchemaIssueKind::AmbiguousAlternatives { matches: values },
            storage: IssueStorage::Prepared(owner),
        }
    }
}
impl Drop for PreparedMatchIndices {
    fn drop(&mut self) {
        if let (Some(mut values), Some(mut lease)) = (self.values.take(), self.lease.take()) {
            values.clear();
            *lease = Some(values);
        }
    }
}

/// A text operation failed through physical memory, its formatter, or UTF-8 validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedDiagnosticTextError {
    /// Actual prepared-memory refusal.
    Memory(crate::memory::MemoryError),
    /// A formatter returned `fmt::Error` without a physical memory refusal.
    Format,
    /// The produced bytes are not valid UTF-8.
    InvalidUtf8,
}
impl std::fmt::Display for PreparedDiagnosticTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory(error) => write!(f, "{error}"),
            Self::Format => f.write_str("diagnostic text formatter failed"),
            Self::InvalidUtf8 => f.write_str("diagnostic text is not UTF-8"),
        }
    }
}
impl std::error::Error for PreparedDiagnosticTextError {}
impl From<crate::memory::MemoryError> for PreparedDiagnosticTextError {
    fn from(error: crate::memory::MemoryError) -> Self {
        Self::Memory(error)
    }
}

/// Immutable UTF-8 text retaining its entire prepared byte array and frame.
pub struct PreparedDiagnosticText(super::SharedPreparedJsonArray<u8>);
impl std::fmt::Debug for PreparedDiagnosticText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_str(), formatter)
    }
}
pub(crate) fn format_diagnostic_text(
    memory: Option<&PreparedIssueMemory>,
    arguments: std::fmt::Arguments<'_>,
) -> Result<PreparedDiagnosticText, SchemaIssue> {
    let memory = memory.ok_or_else(|| {
        SchemaIssue::new(SchemaIssueKind::Memory(
            crate::memory::MemoryError::UnsupportedRunContract,
        ))
    })?;
    let mut builder = memory
        .reserve_text()
        .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?;
    let convert = |error| match error {
        PreparedDiagnosticTextError::Memory(error) => {
            SchemaIssue::new(SchemaIssueKind::Memory(error))
        }
        PreparedDiagnosticTextError::Format | PreparedDiagnosticTextError::InvalidUtf8 => {
            SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
        }
    };
    builder.write_format(arguments).map_err(convert)?;
    builder.finish().map_err(convert)
}

/// Construct a managed unknown-field issue using selected diagnostic backing.
#[doc(hidden)]
#[must_use]
pub fn prepared_unknown_field(
    context: Option<&super::ParseContext>,
    fallback: Option<&PreparedIssueMemory>,
    name: &str,
) -> SchemaIssue {
    let memory = context
        .and_then(super::ParseContext::issue_memory)
        .or(fallback);
    match format_diagnostic_text(memory, format_args!("{name}")) {
        Ok(name) => SchemaIssue::new(SchemaIssueKind::PreparedUnknownField(name)),
        Err(issue) => issue,
    }
}

pub(super) fn prepared_duplicate_field(
    memory: Option<&PreparedIssueMemory>,
    name: &str,
) -> SchemaIssue {
    match format_diagnostic_text(memory, format_args!("{name}")) {
        Ok(name) => SchemaIssue::new(SchemaIssueKind::PreparedDuplicateField(name)),
        Err(issue) => issue,
    }
}
impl PreparedDiagnosticText {
    /// Borrow the validated text while retaining its actual backing.
    #[must_use]
    #[allow(clippy::expect_used)] // Publication validates the immutable bytes before sharing.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(self.0.as_slice()).expect("validated prepared diagnostic text")
    }
    /// Retain another owner of the actual prepared byte array.
    pub fn try_clone(&self) -> Result<Self, crate::memory::MemoryError> {
        Ok(Self(self.0.try_clone()?))
    }
}
impl std::ops::Deref for PreparedDiagnosticText {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

/// Exclusive admitted byte-array builder; failed writes poison publication.
pub struct PreparedDiagnosticTextBuilder {
    bytes: super::PreparedJsonArrayBuilder<u8>,
    failure: Option<PreparedDiagnosticTextError>,
}
impl PreparedDiagnosticTextBuilder {
    /// Copy one complete borrowed UTF-8 string into the admitted backing.
    pub fn write_text(&mut self, text: &str) -> Result<(), PreparedDiagnosticTextError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        let result = (|| {
            let requested = self
                .bytes
                .len()
                .checked_add(text.len())
                .ok_or(crate::memory::MemoryError::ArithmeticOverflow)?;
            if requested > self.bytes.capacity() {
                return Err(crate::memory::MemoryError::ValueTooLarge {
                    requested: crate::memory::Bytes::new(requested),
                    capacity: crate::memory::Bytes::new(self.bytes.capacity()),
                });
            }
            for byte in text.bytes() {
                self.bytes
                    .try_push(byte)
                    .map_err(|_| crate::memory::MemoryError::NoSlot)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.failure = Some(PreparedDiagnosticTextError::Memory(error));
        }
        result.map_err(PreparedDiagnosticTextError::Memory)
    }
    /// Run formatting after admission, distinguishing formatter failure from memory refusal.
    pub fn write_format(
        &mut self,
        arguments: std::fmt::Arguments<'_>,
    ) -> Result<(), PreparedDiagnosticTextError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        let result = std::fmt::write(self, arguments);
        if let Some(error) = self.failure {
            return Err(error);
        }
        if result.is_err() {
            self.failure = Some(PreparedDiagnosticTextError::Format);
            return Err(PreparedDiagnosticTextError::Format);
        }
        Ok(())
    }
    /// Validate UTF-8 and publish only a fully successful builder.
    pub fn finish(self) -> Result<PreparedDiagnosticText, PreparedDiagnosticTextError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        std::str::from_utf8(self.bytes.as_slice())
            .map_err(|_| PreparedDiagnosticTextError::InvalidUtf8)?;
        Ok(PreparedDiagnosticText(self.bytes.publish()))
    }
}
impl std::fmt::Write for PreparedDiagnosticTextBuilder {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.write_text(text).map_err(|_| std::fmt::Error)
    }
    fn write_fmt(&mut self, arguments: std::fmt::Arguments<'_>) -> std::fmt::Result {
        self.write_format(arguments).map_err(|_| std::fmt::Error)
    }
}

struct IssueBacking {
    path: IssuePath,
    children: IssueChildren,
    match_lease: Option<MatchLease>,
}
impl IssueBacking {
    const fn empty() -> Self {
        Self {
            path: IssuePath::Native(Vec::new()),
            children: IssueChildren::Native(Vec::new()),
            match_lease: None,
        }
    }
}
struct IssueOwner(crate::memory::SlotLease<'static, IssueBacking>);
impl Drop for IssueOwner {
    fn drop(&mut self) {
        drop(std::mem::replace(&mut *self.0, IssueBacking::empty()));
    }
}
enum IssueStorage {
    Direct {
        path: IssuePath,
        children: IssueChildren,
    },
    Prepared(IssueOwner),
}
struct PreparedKind {
    kind: Option<SchemaIssueKind>,
    lease: Option<MatchLease>,
}
impl Drop for PreparedKind {
    fn drop(&mut self) {
        if let (Some(SchemaIssueKind::AmbiguousAlternatives { matches }), Some(mut lease)) =
            (self.kind.as_mut(), self.lease.take())
        {
            matches.clear();
            *lease = Some(std::mem::take(matches));
        }
    }
}

/// A typed issue with its encoded-side path, ordered from root to leaf.
pub struct SchemaIssue {
    kind: SchemaIssueKind,
    storage: IssueStorage,
}

impl Clone for SchemaIssue {
    fn clone(&self) -> Self {
        Self {
            kind: self.kind.clone(),
            storage: IssueStorage::Direct {
                path: IssuePath::Native(self.path().to_vec()),
                children: IssueChildren::Native(self.children().to_vec()),
            },
        }
    }
}
impl std::fmt::Debug for SchemaIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaIssue")
            .field("kind", &self.kind)
            .field("path", &self.path())
            .field("children", &self.children())
            .finish()
    }
}
impl PartialEq for SchemaIssue {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.path() == other.path()
            && self.children() == other.children()
    }
}
impl Eq for SchemaIssue {}
impl Drop for SchemaIssue {
    fn drop(&mut self) {
        if let IssueStorage::Prepared(owner) = &mut self.storage
            && let (SchemaIssueKind::AmbiguousAlternatives { matches }, Some(mut lease)) =
                (&mut self.kind, owner.0.match_lease.take())
        {
            matches.clear();
            *lease = Some(std::mem::take(matches));
        }
    }
}

impl SchemaIssue {
    fn parts(&self) -> (&IssuePath, &IssueChildren) {
        match &self.storage {
            IssueStorage::Direct { path, children } => (path, children),
            IssueStorage::Prepared(owner) => (&owner.0.path, &owner.0.children),
        }
    }
    fn parts_mut(&mut self) -> (&mut IssuePath, &mut IssueChildren) {
        match &mut self.storage {
            IssueStorage::Direct { path, children } => (path, children),
            IssueStorage::Prepared(owner) => {
                let backing = &mut *owner.0;
                (&mut backing.path, &mut backing.children)
            }
        }
    }
    fn copy_prepared(
        &self,
        memory: &PreparedIssueMemory,
        prefix: Option<&IssueSegment>,
    ) -> Result<Self, crate::memory::MemoryError> {
        use crate::memory::MemoryError;

        let needs_record = prefix.is_some()
            || !self.path().is_empty()
            || !self.children().is_empty()
            || matches!(&self.kind, SchemaIssueKind::AmbiguousAlternatives { matches } if !matches.is_empty());
        let owner = if needs_record {
            Some(IssueOwner(memory.records.checkout()?))
        } else {
            None
        };

        let mut kind = match &self.kind {
            SchemaIssueKind::AmbiguousAlternatives { matches } if !matches.is_empty() => {
                let mut lease = memory.matches.checkout()?;
                let mut values = lease.take().ok_or(MemoryError::NoSlot)?;
                if values.capacity() < matches.len() {
                    *lease = Some(values);
                    return Err(MemoryError::NoSlot);
                }
                values.extend_from_slice(matches);
                PreparedKind {
                    kind: Some(SchemaIssueKind::AmbiguousAlternatives { matches: values }),
                    lease: Some(lease),
                }
            }
            other @ (SchemaIssueKind::Memory(_)
            | SchemaIssueKind::TypeMismatch { .. }
            | SchemaIssueKind::MissingField(_)
            | SchemaIssueKind::UnknownField(_)
            | SchemaIssueKind::DuplicateField(_)
            | SchemaIssueKind::PreparedUnknownField(_)
            | SchemaIssueKind::PreparedDuplicateField(_)
            | SchemaIssueKind::PreparedJsonSyntax { .. }
            | SchemaIssueKind::PreparedJsonEncoding { .. }
            | SchemaIssueKind::LiteralMismatch
            | SchemaIssueKind::PatternMismatch
            | SchemaIssueKind::InvalidTag
            | SchemaIssueKind::VersionMismatch { .. }
            | SchemaIssueKind::LengthMismatch { .. }
            | SchemaIssueKind::OutOfRange
            | SchemaIssueKind::NonFinite
            | SchemaIssueKind::InvalidValue { .. }
            | SchemaIssueKind::JsonSyntax { .. }
            | SchemaIssueKind::JsonEncoding { .. }
            | SchemaIssueKind::UnsupportedRepresentation { .. }
            | SchemaIssueKind::LimitExceeded { .. }
            | SchemaIssueKind::InvalidDefinition
            | SchemaIssueKind::Multiple
            | SchemaIssueKind::InputConsumed
            | SchemaIssueKind::MissingValidator
            | SchemaIssueKind::MissingWriter
            | SchemaIssueKind::NoAlternative
            | SchemaIssueKind::AmbiguousAlternatives { .. }
            | SchemaIssueKind::Constraint { .. }
            | SchemaIssueKind::ResourceDemand { .. }
            | SchemaIssueKind::PatternBudget
            | SchemaIssueKind::MergeConflict) => PreparedKind {
                kind: Some(other.try_clone_prepared_text()?),
                lease: None,
            },
        };
        let path_len = self
            .path()
            .len()
            .checked_add(usize::from(prefix.is_some()))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let path = if path_len == 0 {
            IssuePath::Native(Vec::new())
        } else {
            let mut path = memory.paths.reserve_shared()?;
            if path.capacity() < path_len {
                return Err(MemoryError::NoSlot);
            }
            if let Some(segment) = prefix {
                path.try_push(segment.try_clone_prepared()?)
                    .map_err(|_| MemoryError::NoSlot)?;
            }
            for segment in self.path() {
                path.try_push(segment.try_clone_prepared()?)
                    .map_err(|_| MemoryError::NoSlot)?;
            }
            IssuePath::Prepared(path.publish())
        };
        let children = if self.children().is_empty() {
            IssueChildren::Native(Vec::new())
        } else {
            let mut children = memory.children.reserve_shared()?;
            if children.capacity() < self.children().len() {
                return Err(MemoryError::NoSlot);
            }
            for child in self.children() {
                let child = child.copy_prepared(memory, prefix)?;
                children.try_push(child).map_err(|_| MemoryError::NoSlot)?;
            }
            IssueChildren::Prepared(children.publish())
        };
        #[allow(clippy::expect_used)]
        // The guard owns its initialized kind until this final transfer.
        let kind_value = kind.kind.take().expect("prepared kind");
        let match_lease = kind.lease.take();
        let storage = match owner {
            Some(mut owner) => {
                *owner.0 = IssueBacking {
                    path,
                    children,
                    match_lease,
                };
                IssueStorage::Prepared(owner)
            }
            None => IssueStorage::Direct { path, children },
        };
        Ok(Self {
            kind: kind_value,
            storage,
        })
    }
    #[must_use]
    /// # Parameters
    /// - `kind` (`SchemaIssueKind`): The kind supplied to this operation.
    ///
    /// # Returns
    /// Type: `Self`.
    pub const fn new(kind: SchemaIssueKind) -> Self {
        Self {
            kind,
            storage: IssueStorage::Direct {
                path: IssuePath::Native(Vec::new()),
                children: IssueChildren::Native(Vec::new()),
            },
        }
    }

    /// Copy this complete issue tree into the supplied prepared backing.
    pub fn try_clone_prepared(
        &self,
        memory: &PreparedIssueMemory,
    ) -> Result<Self, crate::memory::MemoryError> {
        self.copy_prepared(memory, None)
    }

    /// Prefix this issue and its descendants, returning the original intact on refusal.
    #[allow(clippy::result_large_err)] // Returning the original issue preserves its owned backing on refusal.
    pub fn try_at_prepared(
        self,
        segment: IssueSegment,
        memory: &PreparedIssueMemory,
    ) -> Result<Self, (crate::memory::MemoryError, Self)> {
        match self.copy_prepared(memory, Some(&segment)) {
            Ok(issue) => Ok(issue),
            Err(error) => Err((error, self)),
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
        self.parts().0.as_slice()
    }

    #[must_use]
    /// Borrow nested schema issues collected under this parent issue.
    ///
    /// # Returns
    /// Type: `&[Self]`.
    pub fn children(&self) -> &[Self] {
        self.parts().1.as_slice()
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
        *self.parts_mut().1 = IssueChildren::Native(children);
        self
    }

    /// Attach a physically prepared shared child array without copying its contents.
    #[must_use]
    pub fn with_prepared_children(
        mut self,
        children: super::SharedPreparedJsonArray<Self>,
    ) -> Self {
        *self.parts_mut().1 = IssueChildren::Prepared(children);
        self
    }

    /// Attach an actual prepared path owner without copying its segments.
    #[must_use]
    pub fn with_prepared_path(
        mut self,
        path: super::SharedPreparedJsonArray<IssueSegment>,
    ) -> Self {
        *self.parts_mut().0 = IssuePath::Prepared(path);
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
        let mut path = match std::mem::replace(self.parts_mut().0, IssuePath::Native(Vec::new())) {
            IssuePath::Native(path) => path,
            IssuePath::Prepared(path) => path.as_slice().to_vec(),
        };
        path.insert(0, segment.clone());
        *self.parts_mut().0 = IssuePath::Native(path);
        let children = std::mem::replace(self.parts_mut().1, IssueChildren::Native(Vec::new()));
        let children = match children {
            IssueChildren::Native(children) => children
                .into_iter()
                .map(|issue| issue.at(segment.clone()))
                .collect(),
            IssueChildren::Prepared(children) => children
                .as_slice()
                .iter()
                .cloned()
                .map(|issue| issue.at(segment.clone()))
                .collect(),
        };
        *self.parts_mut().1 = IssueChildren::Native(children);
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
        write!(formatter, "{:?} at {:?}", self.kind, self.path())
    }
}

impl std::error::Error for SchemaIssue {}
