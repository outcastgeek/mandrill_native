//! Managed document backing for the shared JSON parser state machine.
use super::*;
use crate::memory::MemoryError;

/// Explicit finite managed JSON documents and per-document parser backing.
/// Physical arena, stack, and text shortages produce memory issues separately
/// from the caller's logical parse limits.
#[derive(Clone, Copy, Debug)]
pub struct PreparedJsonParserCapacity {
    /// Simultaneously retained documents.
    pub documents: usize,
    /// Parsed value nodes per document, including unreachable duplicate values.
    pub nodes_per_document: usize,
    /// Decoded string, property-name and number-token bytes per document.
    pub text_bytes_per_document: usize,
    /// Parser container stack entries per document.
    pub depth_per_document: usize,
}
/// Actual region payload for parser documents; supplied counting-writer backing
/// and caller Arc wrappers are separate. Allocator overhead and alignment rounding
/// are not included.
#[derive(Clone, Copy, Debug)]
pub struct PreparedJsonParserDemand {
    /// Complete selected-region payload and slot-pool bookkeeping.
    pub region_bytes: crate::memory::Bytes,
    /// Fixed node arena bytes across all document slots.
    pub node_bytes: crate::memory::Bytes,
    /// Fixed decoded text bytes across all document slots.
    pub text_bytes: crate::memory::Bytes,
    /// Fixed parser stack bytes across all document slots.
    pub stack_bytes: crate::memory::Bytes,
    /// Document slot-pool metadata and inline record payload bytes.
    pub slot_bytes: crate::memory::Bytes,
}
#[derive(Clone, Copy)]
struct TextSpan {
    start: usize,
    end: usize,
}
#[derive(Clone, Copy)]
enum ManagedKind {
    Null,
    Bool(bool),
    Number(TextSpan),
    String(TextSpan),
    Array,
    Object,
}
#[derive(Clone, Copy)]
struct ManagedNode {
    kind: ManagedKind,
    parent: Option<usize>,
    first: Option<usize>,
    last: Option<usize>,
    next: Option<usize>,
    name: Option<TextSpan>,
    len: usize,
}
type ManagedContainer = Container<usize, usize, TextSpan>;
struct DocumentStorage {
    nodes: crate::memory::FixedVector<'static, ManagedNode>,
    text: crate::memory::Buffer<'static>,
    stack: crate::memory::FixedVector<'static, ManagedContainer>,
}
/// An owned document retaining its actual region-backed arena and text.
///
/// Object views follow retained input order, which may differ from the ordinary
/// `serde_json::Map` ordering. Discarded duplicate nodes and decoded text stay
/// charged to this document until it retires. Caller-owned input and any native
/// diagnostic payloads supplied by callbacks remain separately accounted.
pub struct PreparedJsonDocument {
    storage: Option<crate::memory::SlotLease<'static, DocumentStorage>>,
    root: usize,
}
impl Drop for PreparedJsonDocument {
    fn drop(&mut self) {
        if let Some(storage) = &mut self.storage {
            storage.clear();
        }
    }
}
impl DocumentStorage {
    fn clear(&mut self) {
        self.stack.clear();
        self.nodes.clear();
        self.text.clear();
    }
    fn text(&self, span: TextSpan) -> Option<&str> {
        std::str::from_utf8(self.text.as_slice().get(span.start..span.end)?).ok()
    }
    fn node(&self, index: usize) -> Option<&ManagedNode> {
        self.nodes.as_slice().get(index)
    }
    fn locate_subtree(
        &self,
        root: usize,
        mut current: usize,
        mut issue: SchemaIssue,
        memory: Option<&super::super::super::PreparedIssueMemory>,
    ) -> (SchemaIssue, bool) {
        while current != root {
            let Some(node) = self.node(current) else {
                break;
            };
            let Some(parent_index) = node.parent else {
                break;
            };
            let Some(parent) = self.node(parent_index) else {
                break;
            };
            match parent.kind {
                ManagedKind::Array => {
                    let mut index = 0usize;
                    let mut sibling = parent.first;
                    while let Some(candidate) = sibling {
                        if candidate == current {
                            break;
                        }
                        let Some(node) = self.node(candidate) else {
                            return (issue, false);
                        };
                        sibling = node.next;
                        let Some(next) = index.checked_add(1) else {
                            return (issue, false);
                        };
                        index = next;
                    }
                    if sibling != Some(current) {
                        break;
                    }
                    let (next, refused) =
                        super::super::super::parse_context::selected_issue_at_control(
                            memory,
                            issue,
                            super::super::super::IssueSegment::Index(index),
                        );
                    issue = next;
                    if refused {
                        return (issue, true);
                    }
                }
                ManagedKind::Object => {
                    let Some(name) = node.name.and_then(|span| self.text(span)) else {
                        break;
                    };
                    let segment =
                        match PathMode::Prepared(memory).field(name) {
                            Ok(segment) => segment,
                            Err(error) => return (
                                if super::super::super::prepared_json_validation::physical_cause(
                                    &issue,
                                )
                                .is_some()
                                {
                                    issue
                                } else {
                                    error
                                },
                                true,
                            ),
                        };
                    let (next, refused) =
                        super::super::super::parse_context::selected_issue_at_control(
                            memory, issue, segment,
                        );
                    issue = next;
                    if refused {
                        return (issue, true);
                    }
                }
                ManagedKind::Null
                | ManagedKind::Bool(_)
                | ManagedKind::Number(_)
                | ManagedKind::String(_) => break,
            }
            current = parent_index;
        }
        (issue, false)
    }
}
/// The kind of a borrowed managed JSON value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedJsonKind {
    Null,
    Bool(bool),
    Number,
    String,
    Array,
    Object,
}
/// A value view borrows its document; it cannot survive the backing lease.
#[derive(Clone, Copy)]
pub struct PreparedJsonValue<'a> {
    storage: &'a DocumentStorage,
    index: usize,
}
impl DocumentStorage {
    fn write_value(
        &self,
        root: usize,
        writer: &mut super::super::super::JsonWriter,
    ) -> Result<(), SchemaIssue> {
        let mut current = root;
        loop {
            let node = *self
                .node(current)
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
            match node.kind {
                ManagedKind::Null => writer.null()?,
                ManagedKind::Bool(value) => writer.boolean(value)?,
                ManagedKind::String(span) => {
                    writer
                        .string(self.text(span).ok_or_else(|| {
                            SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
                        })?)?
                }
                ManagedKind::Number(span) => {
                    let token = self
                        .text(span)
                        .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                    writer.managed_number(token)?;
                }
                ManagedKind::Array | ManagedKind::Object => {
                    if matches!(node.kind, ManagedKind::Array) {
                        writer.begin_array()?;
                    } else {
                        writer.begin_object()?;
                    }
                    if let Some(child) = node.first {
                        if matches!(node.kind, ManagedKind::Object) {
                            writer.property(
                                self.text(self.node(child).and_then(|node| node.name).ok_or_else(
                                    || SchemaIssue::new(SchemaIssueKind::InvalidDefinition),
                                )?)
                                .ok_or_else(|| {
                                    SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
                                })?,
                            )?;
                        }
                        current = child;
                        continue;
                    }
                    if matches!(node.kind, ManagedKind::Array) {
                        writer.end_array()?;
                    } else {
                        writer.end_object()?;
                    }
                }
            }
            loop {
                if current == root {
                    return Ok(());
                }
                let node = *self
                    .node(current)
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                let Some(parent_index) = node.parent else {
                    return Ok(());
                };
                let parent = *self
                    .node(parent_index)
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                if let Some(sibling) = node.next {
                    if matches!(parent.kind, ManagedKind::Object) {
                        writer.property(
                            self.text(self.node(sibling).and_then(|node| node.name).ok_or_else(
                                || SchemaIssue::new(SchemaIssueKind::InvalidDefinition),
                            )?)
                            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?,
                        )?;
                    }
                    current = sibling;
                    break;
                }
                match parent.kind {
                    ManagedKind::Array => writer.end_array()?,
                    ManagedKind::Object => writer.end_object()?,
                    ManagedKind::Null
                    | ManagedKind::Bool(_)
                    | ManagedKind::Number(_)
                    | ManagedKind::String(_) => {
                        return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
                    }
                }
                current = parent_index;
            }
        }
    }
}
impl PreparedJsonDocument {
    #[allow(clippy::expect_used)] // A live public document owns its slot until consuming share.
    fn storage(&self) -> &DocumentStorage {
        self.storage.as_deref().expect("live prepared document")
    }
    /// Borrow the document root.
    #[must_use]
    pub fn root(&self) -> PreparedJsonValue<'_> {
        PreparedJsonValue {
            storage: self.storage(),
            index: self.root,
        }
    }
    /// Encode the complete document through the supplied writer.
    pub fn write_to(
        &self,
        writer: &mut super::super::super::JsonWriter,
    ) -> Result<(), SchemaIssue> {
        self.storage().write_value(self.root, writer)
    }
    /// Transfer this actual document slot into a shared, fallibly cloneable view.
    #[allow(clippy::expect_used)] // Consuming self guarantees the unique slot remains present.
    #[must_use]
    pub fn share(mut self) -> SharedPreparedJsonValue {
        let storage = self.storage.take().expect("live prepared document").share();
        SharedPreparedJsonValue {
            storage: Some(storage),
            index: self.root,
        }
    }
}
/// Owned view retaining the complete managed document backing, not only visible bytes.
pub struct SharedPreparedJsonValue {
    storage: Option<crate::memory::SharedSlot<'static, DocumentStorage>>,
    index: usize,
}
impl SharedPreparedJsonValue {
    #[allow(clippy::expect_used)] // The slot is taken only by this owner's Drop.
    fn storage(&self) -> &crate::memory::SharedSlot<'static, DocumentStorage> {
        self.storage.as_ref().expect("live shared prepared value")
    }
    /// Borrow this value while retaining the complete document.
    #[must_use]
    pub fn view(&self) -> PreparedJsonValue<'_> {
        PreparedJsonValue {
            storage: self.storage(),
            index: self.index,
        }
    }
    /// Retain another actual owner of the complete document.
    pub fn try_clone(&self) -> Result<Self, crate::memory::MemoryError> {
        Ok(Self {
            storage: Some(self.storage().try_clone()?),
            index: self.index,
        })
    }
    pub(crate) fn try_first_child(&self) -> Result<Option<Self>, crate::memory::MemoryError> {
        let Some(child) = self.view().first_child() else {
            return Ok(None);
        };
        Ok(Some(Self {
            storage: Some(self.storage().try_clone()?),
            index: child.index,
        }))
    }
    pub(crate) fn try_next_sibling(&self) -> Result<Option<Self>, crate::memory::MemoryError> {
        let Some(child) = self.view().next_sibling() else {
            return Ok(None);
        };
        Ok(Some(Self {
            storage: Some(self.storage().try_clone()?),
            index: child.index,
        }))
    }
    /// Look up and retain an object child, if present.
    pub fn try_get(&self, name: &str) -> Result<Option<Self>, crate::memory::MemoryError> {
        let Some(child) = self.view().get(name) else {
            return Ok(None);
        };
        Ok(Some(Self {
            storage: Some(self.storage().try_clone()?),
            index: child.index,
        }))
    }
    /// Look up and retain an array child, if present.
    pub fn try_get_index(&self, index: usize) -> Result<Option<Self>, crate::memory::MemoryError> {
        let Some(child) = self.view().get_index(index) else {
            return Ok(None);
        };
        Ok(Some(Self {
            storage: Some(self.storage().try_clone()?),
            index: child.index,
        }))
    }
    /// Encode this subtree only, retaining the complete document while writing.
    pub fn write_to(
        &self,
        writer: &mut super::super::super::JsonWriter,
    ) -> Result<(), SchemaIssue> {
        self.storage().write_value(self.index, writer)
    }
}
impl Drop for SharedPreparedJsonValue {
    fn drop(&mut self) {
        if let Some(storage) = self.storage.take()
            && let Some(mut storage) = storage.into_last()
        {
            storage.clear();
        }
    }
}
impl<'a> PreparedJsonValue<'a> {
    /// Encode only this borrowed subtree through the supplied writer.
    pub fn write_to(self, writer: &mut super::super::super::JsonWriter) -> Result<(), SchemaIssue> {
        self.storage.write_value(self.index, writer)
    }

    /// Return this value's kind.
    #[must_use]
    pub fn kind(self) -> Option<PreparedJsonKind> {
        Some(match self.storage.node(self.index)?.kind {
            ManagedKind::Null => PreparedJsonKind::Null,
            ManagedKind::Bool(value) => PreparedJsonKind::Bool(value),
            ManagedKind::Number(_) => PreparedJsonKind::Number,
            ManagedKind::String(_) => PreparedJsonKind::String,
            ManagedKind::Array => PreparedJsonKind::Array,
            ManagedKind::Object => PreparedJsonKind::Object,
        })
    }
    /// Borrow a decoded string value.
    #[must_use]
    pub fn as_str(self) -> Option<&'a str> {
        match self.storage.node(self.index)?.kind {
            ManagedKind::String(span) => self.storage.text(span),
            ManagedKind::Null
            | ManagedKind::Bool(_)
            | ManagedKind::Number(_)
            | ManagedKind::Array
            | ManagedKind::Object => None,
        }
    }
    /// Borrow the original validated number token; it may not be normalized.
    #[must_use]
    pub fn number_token(self) -> Option<&'a str> {
        match self.storage.node(self.index)?.kind {
            ManagedKind::Number(span) => self.storage.text(span),
            ManagedKind::Null
            | ManagedKind::Bool(_)
            | ManagedKind::String(_)
            | ManagedKind::Array
            | ManagedKind::Object => None,
        }
    }
    /// Borrow this child's decoded object property name, when present.
    #[must_use]
    pub fn property_name(self) -> Option<&'a str> {
        self.storage.text(self.storage.node(self.index)?.name?)
    }
    /// First array element or object value in retained input order.
    #[must_use]
    pub fn first_child(self) -> Option<Self> {
        Some(Self {
            storage: self.storage,
            index: self.storage.node(self.index)?.first?,
        })
    }
    /// Next sibling in retained input order.
    #[must_use]
    pub fn next_sibling(self) -> Option<Self> {
        Some(Self {
            storage: self.storage,
            index: self.storage.node(self.index)?.next?,
        })
    }
    /// Look up one retained object property by its decoded name.
    #[must_use]
    pub fn get(self, name: &str) -> Option<Self> {
        if !matches!(self.storage.node(self.index)?.kind, ManagedKind::Object) {
            return None;
        }
        let mut child = self.first_child();
        while let Some(value) = child {
            if value.property_name() == Some(name) {
                return Some(value);
            }
            child = value.next_sibling();
        }
        None
    }
    /// Look up one array element by its zero-based index.
    #[must_use]
    pub fn get_index(self, index: usize) -> Option<Self> {
        if !matches!(self.storage.node(self.index)?.kind, ManagedKind::Array) {
            return None;
        }
        let mut child = self.first_child();
        for _ in 0..index {
            child = child?.next_sibling();
        }
        child
    }
    /// Number of retained direct children.
    #[must_use]
    pub fn len(self) -> usize {
        self.storage.node(self.index).map_or(0, |node| node.len)
    }
    /// Whether this value has no direct children.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}
/// Fixed document slots and caller-supplied normalized counting writer pool.
pub struct PreparedJsonParserPool {
    documents: crate::memory::SlotPool<'static, DocumentStorage>,
    writers: Arc<super::super::super::PreparedJsonWriterPool>,
}

/// The document and normalized counting writer admitted before input is consumed.
pub(crate) struct PreparedJsonParseReservation {
    policy: ParsePolicy,
    document: PreparedJsonDocument,
    writer: super::super::super::JsonWriter,
}

impl PreparedJsonParseReservation {
    pub(crate) fn parse(self, input: &[u8]) -> Result<PreparedJsonDocument, SchemaIssue> {
        PreparedJsonParserPool::parse_admitted(input, self.policy, self.document, self.writer)
    }
}
pub(crate) enum PreparedWriteStageError {
    Admission(SchemaIssue),
    Write(SchemaIssue),
    Parse(SchemaIssue),
}
impl PreparedJsonParserCapacity {
    /// Actual selected-region components; counting-writer backing is separately supplied.
    pub fn demand(self) -> Result<PreparedJsonParserDemand, crate::memory::MemoryError> {
        use crate::memory::{Bytes, MemoryError, SlotPool};
        let nodes = std::alloc::Layout::array::<ManagedNode>(self.nodes_per_document)
            .map_err(|_| MemoryError::InvalidLayout)?
            .size();
        let stack = std::alloc::Layout::array::<ManagedContainer>(self.depth_per_document)
            .map_err(|_| MemoryError::InvalidLayout)?
            .size();
        let node_bytes = Bytes::new(nodes).checked_mul(self.documents)?;
        let text_bytes = Bytes::new(self.text_bytes_per_document).checked_mul(self.documents)?;
        let stack_bytes = Bytes::new(stack).checked_mul(self.documents)?;
        let slot_bytes = SlotPool::<DocumentStorage>::storage_bytes(self.documents)?;
        let region_bytes = slot_bytes
            .checked_add(node_bytes)?
            .checked_add(text_bytes)?
            .checked_add(stack_bytes)?;
        Ok(PreparedJsonParserDemand {
            region_bytes,
            node_bytes,
            text_bytes,
            stack_bytes,
            slot_bytes,
        })
    }
    /// Actual selected-region total; the supplied counting writer is separate.
    pub fn storage_bytes(self) -> Result<crate::memory::Bytes, crate::memory::MemoryError> {
        Ok(self.demand()?.region_bytes)
    }
}
const UTF8_INVALID_SEQUENCE: &str = "invalid utf-8 sequence of ";
const UTF8_BYTES_FROM_INDEX: &str = " bytes from index ";
const UTF8_INCOMPLETE_SEQUENCE: &str = "incomplete utf-8 byte sequence from index ";

impl PreparedJsonParserPool {
    /// Bytes per text owner sufficient for a syntax diagnostic or retained field
    /// name from an input bounded by `input_bytes`.
    #[must_use]
    pub fn diagnostic_text_bytes(input_bytes: std::num::NonZeroUsize) -> usize {
        let input_bytes = input_bytes.get();
        let offset_digits = input_bytes.ilog10() as usize + 1;
        let sequence_digits = size_of::<char>().ilog10() as usize + 1;
        let message_prefix = super::JSON_SYNTAX_AT_BYTE
            .len()
            .max(UTF8_INCOMPLETE_SEQUENCE.len())
            .max(UTF8_INVALID_SEQUENCE.len() + sequence_digits + UTF8_BYTES_FROM_INDEX.len());
        input_bytes.max(message_prefix + offset_digits)
    }

    pub(crate) fn reserve_parse(
        &self,
        policy: ParsePolicy,
    ) -> Result<PreparedJsonParseReservation, crate::memory::MemoryError> {
        let document = PreparedJsonDocument {
            storage: Some(self.documents.try_checkout()?),
            root: 0,
        };
        let writer = self.writers.admit_serde_counting(super::super::super::JsonWriterPolicy {
            bytes: policy.limits.bytes,
            depth: policy.limits.depth,
        })?;
        Ok(PreparedJsonParseReservation { policy, document, writer })
    }
    /// Borrow the diagnostic service selected for its admitted writers.
    #[doc(hidden)]
    #[must_use]
    pub fn issue_memory(&self) -> Option<&super::super::super::PreparedIssueMemory> {
        self.writers.issue_memory()
    }
    pub(crate) fn prepare_name(
        &self,
        name: &str,
        policy: ParsePolicy,
    ) -> Result<super::super::super::PreparedJsonString, SchemaIssue> {
        let mut document = self.admit_document(policy)?;
        let root = document.node(AssemblyKind::String(name), 1)?;
        super::super::super::PreparedJsonString::new(document.finish_merge_for_writer(root)?)
    }
    pub(crate) fn copy_native_template(
        &self,
        value: &JsonValue,
        policy: ParsePolicy,
    ) -> Result<SharedPreparedJsonValue, SchemaIssue> {
        let mut document = self.admit_document(policy)?;
        let root = document.copy_native(value, 1)?;
        document.finish(root)
    }
    pub(crate) fn assemble_bytes(
        &self,
        values: &[u8],
        policy: ParsePolicy,
    ) -> Result<SharedPreparedJsonValue, SchemaIssue> {
        if values.len() > policy.limits.collection.get() {
            return Err(exceeded(SchemaResource::Width));
        }
        let required_nodes = values
            .len()
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Steps))?;
        if required_nodes > policy.limits.nodes.get() {
            return Err(exceeded(SchemaResource::Steps));
        }
        if !values.is_empty() && policy.limits.depth.get() < 2 {
            return Err(exceeded(SchemaResource::Depth));
        }
        let mut document = self.reserve_document()?;
        let mut writer = self.counting_writer(policy)?;
        let mut digits = 0usize;
        for value in values {
            let mut buffer = std::fmt::NumBuffer::new();
            digits = digits
                .checked_add(value.format_into(&mut buffer).len())
                .ok_or_else(|| exceeded(SchemaResource::Bytes))?;
        }
        let encoded = digits
            .checked_add(values.len().saturating_sub(1))
            .and_then(|n| n.checked_add(2))
            .ok_or_else(|| exceeded(SchemaResource::Bytes))?;
        if encoded > policy.limits.bytes.get() {
            return Err(exceeded(SchemaResource::Bytes));
        }
        let storage = document
            .storage
            .as_deref_mut()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        if required_nodes > storage.nodes.capacity() {
            return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                MemoryError::NoSlot,
            )));
        }
        if digits > storage.text.capacity().get() {
            return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                MemoryError::ValueTooLarge {
                    requested: crate::memory::Bytes::new(digits),
                    capacity: storage.text.capacity(),
                },
            )));
        }
        let mut arena = Arena {
            nodes: &mut storage.nodes,
            text: &mut storage.text,
        };
        document.root = arena.add(ManagedKind::Array)?;
        for value in values {
            let mut buffer = std::fmt::NumBuffer::new();
            let start = arena.text.len();
            arena
                .text
                .extend_from_slice(value.format_into(&mut buffer).as_bytes())
                .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?;
            let span = TextSpan {
                start,
                end: arena.text.len(),
            };
            let child = arena.add(ManagedKind::Number(span))?;
            arena.append(document.root, child, None)?;
        }
        document.write_to(&mut writer)?;
        writer.finish_sink()?;
        Ok(document.share())
    }
    /// Prepare all arena/text/stack slots; writer pool backing is separately supplied.
    pub fn prepare(
        memory: &crate::memory::PreparedMemory<'static>,
        region: crate::memory::RegionId,
        capacity: PreparedJsonParserCapacity,
        writers: Arc<super::super::super::PreparedJsonWriterPool>,
    ) -> Result<Self, crate::memory::MemoryError> {
        let _ = capacity.storage_bytes()?;
        let documents = memory.slots(region, capacity.documents, |_| {
            Ok(DocumentStorage {
                nodes: crate::memory::FixedVector::prepare(
                    memory.allocator(region)?,
                    capacity.nodes_per_document,
                )?,
                text: memory.buffer(
                    region,
                    crate::memory::Bytes::new(capacity.text_bytes_per_document),
                )?,
                stack: crate::memory::FixedVector::prepare(
                    memory.allocator(region)?,
                    capacity.depth_per_document,
                )?,
            })
        })?;
        Ok(Self { documents, writers })
    }
    /// Parse text into an owned managed document and check normalized byte demand.
    pub fn parse(
        &self,
        text: &str,
        policy: ParsePolicy,
    ) -> Result<PreparedJsonDocument, SchemaIssue> {
        self.parse_bytes(text.as_bytes(), policy)
    }
    /// Admit all physical owners before validating and parsing the UTF-8 input.
    pub fn parse_bytes(
        &self,
        input: &[u8],
        policy: ParsePolicy,
    ) -> Result<PreparedJsonDocument, SchemaIssue> {
        let document = self.reserve_document()?;
        let writer = self.counting_writer(policy)?;
        Self::parse_admitted(input, policy, document, writer)
    }
    fn reserve_document(&self) -> Result<PreparedJsonDocument, SchemaIssue> {
        let storage = self
            .documents
            .checkout()
            .map_err(|_| exceeded(SchemaResource::JsonDocuments))?;
        Ok(PreparedJsonDocument {
            storage: Some(storage),
            root: 0,
        })
    }
    fn counting_writer(
        &self,
        policy: ParsePolicy,
    ) -> Result<super::super::super::JsonWriter, SchemaIssue> {
        self.writers
            .admit_serde_counting(super::super::super::JsonWriterPolicy {
                bytes: policy.limits.bytes,
                depth: policy.limits.depth,
            })
            .map_err(|_| exceeded(SchemaResource::RepresentationChecks))
    }
    fn parse_admitted(
        input: &[u8],
        policy: ParsePolicy,
        mut document: PreparedJsonDocument,
        mut writer: super::super::super::JsonWriter,
    ) -> Result<PreparedJsonDocument, SchemaIssue> {
        if input.len() > policy.limits.bytes.get() {
            return Err(exceeded(SchemaResource::Bytes));
        }
        let text = std::str::from_utf8(input).map_err(|error| {
            let offset = error.valid_up_to();
            let memory = writer.issue_memory();
            let detail = if let Some(width) = error.error_len() {
                super::super::super::schema_issue::format_diagnostic_text(
                    memory,
                    format_args!("{UTF8_INVALID_SEQUENCE}{width}{UTF8_BYTES_FROM_INDEX}{offset}"),
                )
            } else {
                super::super::super::schema_issue::format_diagnostic_text(
                    memory,
                    format_args!("{UTF8_INCOMPLETE_SEQUENCE}{offset}"),
                )
            };
            match detail {
                Ok(detail) => SchemaIssue::new(SchemaIssueKind::PreparedJsonSyntax { detail }),
                Err(issue) => issue,
            }
        })?;
        let root = {
            let storage = document
                .storage
                .as_deref_mut()
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
            let mut arena = Arena {
                nodes: &mut storage.nodes,
                text: &mut storage.text,
            };
            parse_machine(
                text,
                policy,
                &mut arena,
                &mut storage.stack,
                PathMode::Prepared(writer.issue_memory()),
            )?
        };
        document.root = root;
        document.write_to(&mut writer)?;
        writer.finish_sink()?;
        Ok(document)
    }
    /// Assemble one managed array from encoded children without a DOM conversion.
    /// The document and normalized counting writer are admitted before any child factory.
    pub fn assemble_array(
        &self,
        count: usize,
        policy: ParsePolicy,
        mut child: impl FnMut(usize) -> Result<SharedPreparedJsonValue, SchemaIssue>,
    ) -> Result<SharedPreparedJsonValue, SchemaIssue> {
        if count > policy.limits.collection.get() {
            return Err(exceeded(SchemaResource::Width));
        }
        let mut document = self.reserve_document()?;
        let mut writer = self.counting_writer(policy)?;
        let minimum_nodes = count
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Steps))?;
        if minimum_nodes > policy.limits.nodes.get() {
            return Err(exceeded(SchemaResource::Steps));
        }
        if count > 0 && policy.limits.depth.get() < 2 {
            return Err(exceeded(SchemaResource::Depth));
        }
        {
            let storage = document
                .storage
                .as_deref_mut()
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
            if minimum_nodes > storage.nodes.capacity() {
                return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                    MemoryError::NoSlot,
                )));
            }
            let mut arena = Arena {
                nodes: &mut storage.nodes,
                text: &mut storage.text,
            };
            document.root = arena.add(ManagedKind::Array)?;
        }
        let mut nodes = 1usize;
        for index in 0..count {
            let value = child(index)?;
            let storage = document
                .storage
                .as_deref_mut()
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
            let mut arena = Arena {
                nodes: &mut storage.nodes,
                text: &mut storage.text,
            };
            arena.copy_subtree(
                value.storage(),
                value.index,
                document.root,
                None,
                &mut nodes,
                CopySettings {
                    limits: policy.limits,
                    unpathed_logical_limits: true,
                    enforce_logical_limits: true,
                    issues: writer.issue_memory(),
                    outer_index: Some(index),
                },
            )?;
        }
        document.write_to(&mut writer)?;
        writer.finish_sink()?;
        Ok(document.share())
    }
    /// Encode with a genuine direct-writer witness, then parse into one preadmitted document.
    /// This bounded double pass retains one document and two writer records concurrently;
    /// the witness also needs its physically prepared codec-boundary snapshots.
    pub fn encode_with<A: Send + 'static, Encoded: Send + 'static>(
        &self,
        codec: &super::super::super::PureCodec<A, Encoded>,
        value: &A,
        policy: ParsePolicy,
    ) -> Result<SharedPreparedJsonValue, SchemaIssue> {
        let document = self.reserve_document()?;
        let mut output = self
            .writers
            .admit_buffered(super::super::super::JsonWriterPolicy {
                bytes: policy.limits.bytes,
                depth: policy.limits.depth,
            })
            .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?;
        let counting = self.counting_writer(policy)?;
        codec.write_into(value, &mut output)?;
        let output = output.finish_prepared()?;
        let document = Self::parse_admitted(output.as_bytes(), policy, document, counting)?;
        Ok(document.share())
    }
    pub(crate) fn encode_with_writer_policy<A: Send + 'static, Encoded: Send + 'static>(
        &self,
        codec: &super::super::super::PureCodec<A, Encoded>,
        value: &A,
        policy: ParsePolicy,
        writer_policy: super::super::super::JsonWriterPolicy,
    ) -> Result<SharedPreparedJsonValue, PreparedWriteStageError> {
        let document = self
            .reserve_document()
            .map_err(PreparedWriteStageError::Admission)?;
        let mut output = self
            .writers
            .admit_buffered(writer_policy)
            .map_err(|error| {
                PreparedWriteStageError::Admission(SchemaIssue::new(SchemaIssueKind::Memory(error)))
            })?;
        let counting = self
            .counting_writer(policy)
            .map_err(PreparedWriteStageError::Admission)?;
        codec
            .write_into(value, &mut output)
            .map_err(PreparedWriteStageError::Write)?;
        let output = output
            .finish_prepared()
            .map_err(PreparedWriteStageError::Write)?;
        let document = Self::parse_admitted(output.as_bytes(), policy, document, counting)
            .map_err(PreparedWriteStageError::Parse)?;
        Ok(document.share())
    }
}
struct Arena<'a> {
    nodes: &'a mut crate::memory::FixedVector<'static, ManagedNode>,
    text: &'a mut crate::memory::Buffer<'static>,
}
#[derive(Clone, Copy)]
struct CopySettings<'a> {
    limits: ParseLimits,
    unpathed_logical_limits: bool,
    enforce_logical_limits: bool,
    issues: Option<&'a super::super::super::PreparedIssueMemory>,
    outer_index: Option<usize>,
}
pub(crate) struct PreparedDocumentAssembly {
    document: PreparedJsonDocument,
    writer: super::super::super::JsonWriter,
    policy: ParsePolicy,
}
pub(crate) struct PreparedArrayAssembly {
    document: PreparedDocumentAssembly,
    root: usize,
    nodes: usize,
    count: usize,
}
#[derive(Clone, Copy)]
pub(crate) enum AssemblyKind<'a> {
    Null,
    Bool(bool),
    Number(&'a str),
    String(&'a str),
    Array,
    Object,
}
impl PreparedJsonParserPool {
    pub(crate) fn admit_array(
        &self,
        policy: ParsePolicy,
    ) -> Result<PreparedArrayAssembly, SchemaIssue> {
        let mut document = self.admit_document(policy)?;
        let root = document.node(AssemblyKind::Array, 1)?;
        Ok(PreparedArrayAssembly {
            document,
            root,
            nodes: 1,
            count: 0,
        })
    }
    pub(crate) fn admit_document(
        &self,
        policy: ParsePolicy,
    ) -> Result<PreparedDocumentAssembly, SchemaIssue> {
        Ok(PreparedDocumentAssembly {
            document: self.reserve_document()?,
            writer: self.counting_writer(policy)?,
            policy,
        })
    }
}
impl PreparedArrayAssembly {
    pub(crate) fn preflight_count(&self, count: usize) -> Result<(), SchemaIssue> {
        if count > self.document.policy.limits.collection.get() {
            return Err(exceeded(SchemaResource::Width));
        }
        let nodes = count
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Steps))?;
        if nodes > self.document.policy.limits.nodes.get() {
            return Err(exceeded(SchemaResource::Steps));
        }
        if count > 0 && self.document.policy.limits.depth.get() < 2 {
            return Err(exceeded(SchemaResource::Depth));
        }
        let storage = self
            .document
            .document
            .storage
            .as_deref()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        if nodes > storage.nodes.capacity() {
            return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                MemoryError::NoSlot,
            )));
        }
        Ok(())
    }
    pub(crate) fn push(
        &mut self,
        produce: impl FnOnce() -> Result<SharedPreparedJsonValue, SchemaIssue>,
    ) -> Result<(), SchemaIssue> {
        self.preflight()?;
        let value = produce()?;
        self.push_value(value)
    }
    pub(crate) fn preflight(&self) -> Result<(), SchemaIssue> {
        let next = self
            .count
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Width))?;
        if next > self.document.policy.limits.collection.get() {
            return Err(exceeded(SchemaResource::Width));
        }
        if self.document.policy.limits.depth.get() < 2 {
            return Err(exceeded(SchemaResource::Depth));
        }
        if self
            .nodes
            .checked_add(1)
            .is_none_or(|n| n > self.document.policy.limits.nodes.get())
        {
            return Err(exceeded(SchemaResource::Steps));
        }
        let storage = self
            .document
            .document
            .storage
            .as_deref()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        if storage.nodes.len() == storage.nodes.capacity() {
            return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                MemoryError::NoSlot,
            )));
        }
        Ok(())
    }
    pub(crate) fn push_value(&mut self, value: SharedPreparedJsonValue) -> Result<(), SchemaIssue> {
        self.preflight()?;
        let next = self
            .count
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Width))?;
        let storage = self
            .document
            .document
            .storage
            .as_deref_mut()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        let mut arena = Arena {
            nodes: &mut storage.nodes,
            text: &mut storage.text,
        };
        arena.copy_subtree(
            value.storage(),
            value.index,
            self.root,
            None,
            &mut self.nodes,
            CopySettings {
                limits: self.document.policy.limits,
                unpathed_logical_limits: true,
                enforce_logical_limits: true,
                issues: self.document.writer.issue_memory(),
                outer_index: None,
            },
        )?;
        self.count = next;
        Ok(())
    }
    pub(crate) fn finish(self) -> Result<SharedPreparedJsonValue, SchemaIssue> {
        self.document.finish(self.root)
    }
}
impl PreparedDocumentAssembly {
    pub(crate) fn merge_object_insert(
        &mut self,
        target: usize,
        name: &str,
        value: PreparedJsonValue<'_>,
        fallback: Option<&super::super::super::PreparedIssueMemory>,
    ) -> Result<(), SchemaIssue> {
        self.merge_append(target, value, Some(name), fallback)
    }
    /// Initialize an admitted merge document from a previously checked managed value.
    pub(crate) fn merge_root(
        &mut self,
        source: PreparedJsonValue<'_>,
        fallback: Option<&super::super::super::PreparedIssueMemory>,
    ) -> Result<usize, SchemaIssue> {
        let kind = match source
            .kind()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?
        {
            PreparedJsonKind::Null => AssemblyKind::Null,
            PreparedJsonKind::Bool(value) => AssemblyKind::Bool(value),
            PreparedJsonKind::Number => AssemblyKind::Number(
                source
                    .number_token()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?,
            ),
            PreparedJsonKind::String => AssemblyKind::String(
                source
                    .as_str()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?,
            ),
            PreparedJsonKind::Array => AssemblyKind::Array,
            PreparedJsonKind::Object => AssemblyKind::Object,
        };
        let root = self.node(kind, 1)?;
        let mut child = source.first_child();
        while let Some(value) = child {
            self.merge_append(root, value, value.property_name(), fallback)?;
            child = value.next_sibling();
        }
        Ok(root)
    }
    fn merge_append(
        &mut self,
        target: usize,
        value: PreparedJsonValue<'_>,
        name: Option<&str>,
        fallback: Option<&super::super::super::PreparedIssueMemory>,
    ) -> Result<(), SchemaIssue> {
        let storage = self
            .document
            .storage
            .as_deref_mut()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        let mut nodes = storage.nodes.len();
        let mut arena = Arena {
            nodes: &mut storage.nodes,
            text: &mut storage.text,
        };
        let span = if let Some(name) = name {
            let start = arena.text.len();
            arena
                .text
                .extend_from_slice(name.as_bytes())
                .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?;
            Some(TextSpan {
                start,
                end: arena.text.len(),
            })
        } else {
            None
        };
        arena.copy_subtree(
            value.storage,
            value.index,
            target,
            span,
            &mut nodes,
            CopySettings {
                limits: self.policy.limits,
                unpathed_logical_limits: true,
                enforce_logical_limits: false,
                issues: self.writer.issue_memory().or(fallback),
                outer_index: None,
            },
        )
    }
    /// Merge a checked branch into the retained target; merged logical limits are checked after all branches.
    pub(crate) fn merge_value(
        &mut self,
        target: usize,
        source: PreparedJsonValue<'_>,
        fallback: Option<&super::super::super::PreparedIssueMemory>,
    ) -> Result<(), SchemaIssue> {
        let current = self.view(target)?;
        if current.kind() == Some(PreparedJsonKind::Object)
            && source.kind() == Some(PreparedJsonKind::Object)
        {
            let mut child = source.first_child();
            while let Some(value) = child {
                let name = value
                    .property_name()
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                let existing = self.view(target)?.get(name).map(|entry| entry.index);
                if let Some(existing) = existing {
                    if let Err(issue) = self.merge_value(existing, value, fallback) {
                        let memory = self.writer.issue_memory().or(fallback);
                        let segment = match PathMode::Prepared(memory).field(name) {
                            Ok(segment) => segment,
                            Err(error) => return Err(
                                if super::super::super::prepared_json_validation::physical_cause(
                                    &issue,
                                )
                                .is_some()
                                {
                                    issue
                                } else {
                                    error
                                },
                            ),
                        };
                        return Err(
                            super::super::super::parse_context::selected_issue_at_control(
                                memory, issue, segment,
                            )
                            .0,
                        );
                    }
                } else {
                    self.merge_append(target, value, Some(name), fallback)?;
                }
                child = value.next_sibling();
            }
            Ok(())
        } else if super::super::super::managed_literal::numeric_structural_equal(current, source)? {
            Ok(())
        } else {
            Err(SchemaIssue::new(SchemaIssueKind::MergeConflict))
        }
    }
    fn copy_native(&mut self, value: &JsonValue, depth: usize) -> Result<usize, SchemaIssue> {
        let kind = match value {
            JsonValue::Null => AssemblyKind::Null,
            JsonValue::Bool(value) => AssemblyKind::Bool(*value),
            JsonValue::Number(value) => AssemblyKind::Number(value.as_str()),
            JsonValue::String(value) => AssemblyKind::String(value),
            JsonValue::Array(_) => AssemblyKind::Array,
            JsonValue::Object(_) => AssemblyKind::Object,
        };
        let root = self.node(kind, depth)?;
        match value {
            JsonValue::Array(values) => {
                if values.len() > self.policy.limits.collection.get() {
                    return Err(exceeded(SchemaResource::Width));
                }
                for value in values {
                    let next = depth
                        .checked_add(1)
                        .ok_or_else(|| exceeded(SchemaResource::Depth))?;
                    let child = self.copy_native(value, next)?;
                    self.append(root, child, None)?;
                }
            }
            JsonValue::Object(values) => {
                if values.len() > self.policy.limits.collection.get() {
                    return Err(exceeded(SchemaResource::Width));
                }
                for (name, value) in values {
                    let next = depth
                        .checked_add(1)
                        .ok_or_else(|| exceeded(SchemaResource::Depth))?;
                    let child = self.copy_native(value, next)?;
                    self.append(root, child, Some(name))?;
                }
            }
            JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => {}
        }
        Ok(root)
    }
    pub(crate) fn node(
        &mut self,
        kind: AssemblyKind<'_>,
        depth: usize,
    ) -> Result<usize, SchemaIssue> {
        if let AssemblyKind::Number(text) = kind
            && super::super::super::managed_number::ManagedNumber::parse(text).is_none()
        {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        if depth > self.policy.limits.depth.get() {
            return Err(exceeded(SchemaResource::Depth));
        }
        let storage = self
            .document
            .storage
            .as_deref_mut()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        if storage.nodes.len() >= self.policy.limits.nodes.get() {
            return Err(exceeded(SchemaResource::Steps));
        }
        let mut arena = Arena {
            nodes: &mut storage.nodes,
            text: &mut storage.text,
        };
        let managed = match kind {
            AssemblyKind::Null => ManagedKind::Null,
            AssemblyKind::Bool(value) => ManagedKind::Bool(value),
            AssemblyKind::Array => ManagedKind::Array,
            AssemblyKind::Object => ManagedKind::Object,
            AssemblyKind::Number(text) | AssemblyKind::String(text) => {
                let start = arena.text.len();
                arena
                    .text
                    .extend_from_slice(text.as_bytes())
                    .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?;
                let span = TextSpan {
                    start,
                    end: arena.text.len(),
                };
                match kind {
                    AssemblyKind::Number(_) => ManagedKind::Number(span),
                    AssemblyKind::String(_) => ManagedKind::String(span),
                    AssemblyKind::Null
                    | AssemblyKind::Bool(_)
                    | AssemblyKind::Array
                    | AssemblyKind::Object => {
                        return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
                    }
                }
            }
        };
        arena.add(managed)
    }
    pub(crate) fn append(
        &mut self,
        parent: usize,
        child: usize,
        name: Option<&str>,
    ) -> Result<(), SchemaIssue> {
        let storage = self
            .document
            .storage
            .as_deref_mut()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        let mut arena = Arena {
            nodes: &mut storage.nodes,
            text: &mut storage.text,
        };
        let parent_node = *arena.node(parent)?;
        if child <= parent
            || arena.node(child)?.parent.is_some()
            || parent_node.len >= self.policy.limits.collection.get()
        {
            return Err(if parent_node.len >= self.policy.limits.collection.get() {
                exceeded(SchemaResource::Width)
            } else {
                SchemaIssue::new(SchemaIssueKind::InvalidDefinition)
            });
        }
        if !matches!(
            (parent_node.kind, name),
            (ManagedKind::Array, None) | (ManagedKind::Object, Some(_))
        ) {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        let name = if let Some(name) = name {
            let start = arena.text.len();
            arena
                .text
                .extend_from_slice(name.as_bytes())
                .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?;
            Some(TextSpan {
                start,
                end: arena.text.len(),
            })
        } else {
            None
        };
        arena.append(parent, child, name)
    }
    pub(crate) fn view(&self, index: usize) -> Result<PreparedJsonValue<'_>, SchemaIssue> {
        let storage = self
            .document
            .storage
            .as_deref()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        storage
            .node(index)
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        Ok(PreparedJsonValue { storage, index })
    }
    pub(crate) fn finish(mut self, root: usize) -> Result<SharedPreparedJsonValue, SchemaIssue> {
        let storage = self
            .document
            .storage
            .as_deref()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        if storage.node(root).is_none_or(|node| node.parent.is_some()) {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        self.document.root = root;
        self.document.write_to(&mut self.writer).map_err(|error| {
            if matches!(
                error.kind(),
                SchemaIssueKind::LimitExceeded {
                    resource: SchemaResource::JsonDepth
                }
            ) {
                exceeded(SchemaResource::Depth)
            } else {
                error
            }
        })?;
        self.writer.finish_sink()?;
        Ok(self.document.share())
    }
    /// Validate the completed merge in legacy traversal order using its admitted writer.
    pub(crate) fn finish_merge(
        self,
        root: usize,
        validation: &super::super::super::PreparedJsonValidationPool,
    ) -> Result<SharedPreparedJsonValue, SchemaIssue> {
        let Self {
            mut document,
            writer,
            policy,
        } = self;
        let storage = document
            .storage
            .as_deref()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        if storage.node(root).is_none_or(|node| node.parent.is_some()) {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        validation.check_parse_with_writer(
            PreparedJsonValue {
                storage,
                index: root,
            },
            &policy.limits,
            writer,
        )?;
        document.root = root;
        Ok(document.share())
    }
    /// Publish a writer-path merge; the caller's actual writer enforces its output policy.
    pub(crate) fn finish_merge_for_writer(
        mut self,
        root: usize,
    ) -> Result<SharedPreparedJsonValue, SchemaIssue> {
        let storage = self
            .document
            .storage
            .as_deref()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        if storage.node(root).is_none_or(|node| node.parent.is_some()) {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        self.document.root = root;
        Ok(self.document.share())
    }
}
#[doc(hidden)]
pub struct PreparedObjectAssembly {
    document: PreparedJsonDocument,
    writer: super::super::super::JsonWriter,
    policy: ParsePolicy,
    nodes: usize,
    fields: usize,
}
impl PreparedJsonParserPool {
    #[doc(hidden)]
    pub fn admit_object(&self, policy: ParsePolicy) -> Result<PreparedObjectAssembly, SchemaIssue> {
        let mut document = self.reserve_document()?;
        let writer = self.counting_writer(policy)?;
        let storage = document
            .storage
            .as_deref_mut()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        let mut arena = Arena {
            nodes: &mut storage.nodes,
            text: &mut storage.text,
        };
        document.root = arena.add(ManagedKind::Object)?;
        Ok(PreparedObjectAssembly {
            document,
            writer,
            policy,
            nodes: 1,
            fields: 0,
        })
    }
}
impl PreparedObjectAssembly {
    pub(crate) fn preflight_names<'a>(
        &self,
        names: impl Iterator<Item = &'a str>,
    ) -> Result<usize, SchemaIssue> {
        let storage = self
            .document
            .storage
            .as_deref()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        let mut count = 0usize;
        let mut bytes = storage.text.len();
        for name in names {
            count = count
                .checked_add(1)
                .ok_or_else(|| exceeded(SchemaResource::Width))?;
            if count > self.policy.limits.collection.get() {
                return Err(exceeded(SchemaResource::Width));
            }
            bytes = bytes.checked_add(name.len()).ok_or_else(|| {
                SchemaIssue::new(SchemaIssueKind::Memory(MemoryError::ArithmeticOverflow))
            })?;
        }
        let nodes = count
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Steps))?;
        if nodes > self.policy.limits.nodes.get() {
            return Err(exceeded(SchemaResource::Steps));
        }
        if count > 0 && self.policy.limits.depth.get() < 2 {
            return Err(exceeded(SchemaResource::Depth));
        }
        if nodes > storage.nodes.capacity() {
            return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                MemoryError::NoSlot,
            )));
        }
        if bytes > storage.text.capacity().get() {
            return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                MemoryError::ValueTooLarge {
                    requested: crate::memory::Bytes::new(bytes),
                    capacity: storage.text.capacity(),
                },
            )));
        }
        Ok(count)
    }
    pub(crate) fn insert_string(&mut self, name: &str, value: &str) -> Result<(), SchemaIssue> {
        let next = self
            .fields
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Width))?;
        if next > self.policy.limits.collection.get() {
            return Err(exceeded(SchemaResource::Width));
        }
        if self.policy.limits.depth.get() < 2 {
            return Err(exceeded(SchemaResource::Depth));
        }
        if self
            .nodes
            .checked_add(1)
            .is_none_or(|n| n > self.policy.limits.nodes.get())
        {
            return Err(exceeded(SchemaResource::Steps));
        }
        let storage = self
            .document
            .storage
            .as_deref_mut()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        if storage.nodes.len() == storage.nodes.capacity() {
            return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                MemoryError::NoSlot,
            )));
        }
        let name_start = storage.text.len();
        storage
            .text
            .extend_from_slice(name.as_bytes())
            .map_err(|e| SchemaIssue::new(SchemaIssueKind::Memory(e)))?;
        let name = TextSpan {
            start: name_start,
            end: storage.text.len(),
        };
        let value_start = storage.text.len();
        storage
            .text
            .extend_from_slice(value.as_bytes())
            .map_err(|e| SchemaIssue::new(SchemaIssueKind::Memory(e)))?;
        let value = TextSpan {
            start: value_start,
            end: storage.text.len(),
        };
        let mut arena = Arena {
            nodes: &mut storage.nodes,
            text: &mut storage.text,
        };
        let child = arena.add(ManagedKind::String(value))?;
        arena.append(self.document.root, child, Some(name))?;
        self.nodes += 1;
        self.fields = next;
        Ok(())
    }
    pub(crate) fn preflight(&self, name: &str) -> Result<(), SchemaIssue> {
        let next = self
            .fields
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Width))?;
        if next > self.policy.limits.collection.get() {
            return Err(exceeded(SchemaResource::Width));
        }
        if self.policy.limits.depth.get() < 2 {
            return Err(exceeded(SchemaResource::Depth));
        }
        if self
            .nodes
            .checked_add(1)
            .is_none_or(|n| n > self.policy.limits.nodes.get())
        {
            return Err(exceeded(SchemaResource::Steps));
        }
        let storage = self
            .document
            .storage
            .as_deref()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        if storage.nodes.len() == storage.nodes.capacity() {
            return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                MemoryError::NoSlot,
            )));
        }
        let requested = storage.text.len().checked_add(name.len()).ok_or_else(|| {
            SchemaIssue::new(SchemaIssueKind::Memory(MemoryError::ArithmeticOverflow))
        })?;
        if requested > storage.text.capacity().get() {
            return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                MemoryError::ValueTooLarge {
                    requested: crate::memory::Bytes::new(requested),
                    capacity: storage.text.capacity(),
                },
            )));
        }
        Ok(())
    }
    pub(crate) fn insert_value(
        &mut self,
        name: &str,
        value: SharedPreparedJsonValue,
    ) -> Result<(), SchemaIssue> {
        self.preflight(name)?;
        self.insert(name, || Ok(value))
    }
    #[doc(hidden)]
    pub fn insert(
        &mut self,
        name: &str,
        encode: impl FnOnce() -> Result<SharedPreparedJsonValue, SchemaIssue>,
    ) -> Result<(), SchemaIssue> {
        let next = self
            .fields
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Width))?;
        if next > self.policy.limits.collection.get() {
            return Err(exceeded(SchemaResource::Width));
        }
        if self.policy.limits.depth.get() < 2 {
            return Err(exceeded(SchemaResource::Depth));
        }
        if self
            .nodes
            .checked_add(1)
            .is_none_or(|n| n > self.policy.limits.nodes.get())
        {
            return Err(exceeded(SchemaResource::Steps));
        }
        let storage = self
            .document
            .storage
            .as_deref_mut()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        if storage.nodes.len() == storage.nodes.capacity() {
            return Err(SchemaIssue::new(SchemaIssueKind::Memory(
                MemoryError::NoSlot,
            )));
        }
        let start = storage.text.len();
        storage
            .text
            .extend_from_slice(name.as_bytes())
            .map_err(|e| SchemaIssue::new(SchemaIssueKind::Memory(e)))?;
        let span = TextSpan {
            start,
            end: storage.text.len(),
        };
        let value = encode()?;
        let storage = self
            .document
            .storage
            .as_deref_mut()
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        let mut arena = Arena {
            nodes: &mut storage.nodes,
            text: &mut storage.text,
        };
        arena.copy_subtree(
            value.storage(),
            value.index,
            self.document.root,
            Some(span),
            &mut self.nodes,
            CopySettings {
                limits: self.policy.limits,
                unpathed_logical_limits: true,
                enforce_logical_limits: true,
                issues: self.writer.issue_memory(),
                outer_index: None,
            },
        )?;
        self.fields = next;
        Ok(())
    }
    #[doc(hidden)]
    pub fn finish(mut self) -> Result<SharedPreparedJsonValue, SchemaIssue> {
        self.document.write_to(&mut self.writer)?;
        self.writer.finish_sink()?;
        Ok(self.document.share())
    }
}
impl Arena<'_> {
    fn node(&self, index: usize) -> Result<&ManagedNode, SchemaIssue> {
        self.nodes
            .as_slice()
            .get(index)
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))
    }
    fn node_mut(&mut self, index: usize) -> Result<&mut ManagedNode, SchemaIssue> {
        self.nodes
            .as_mut_slice()
            .get_mut(index)
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))
    }
    fn text(&self, span: TextSpan) -> Result<&str, SchemaIssue> {
        std::str::from_utf8(
            self.text
                .as_slice()
                .get(span.start..span.end)
                .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?,
        )
        .map_err(|_| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))
    }
    fn add(&mut self, kind: ManagedKind) -> Result<usize, SchemaIssue> {
        let index = self.nodes.len();
        self.nodes
            .push(ManagedNode {
                kind,
                parent: None,
                first: None,
                last: None,
                next: None,
                name: None,
                len: 0,
            })
            .map_err(|_| SchemaIssue::new(SchemaIssueKind::Memory(MemoryError::NoSlot)))?;
        Ok(index)
    }
    fn copy_span(
        &mut self,
        source: &DocumentStorage,
        span: TextSpan,
    ) -> Result<TextSpan, SchemaIssue> {
        let bytes = source
            .text
            .as_slice()
            .get(span.start..span.end)
            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        let start = self.text.len();
        self.text
            .extend_from_slice(bytes)
            .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))?;
        Ok(TextSpan {
            start,
            end: self.text.len(),
        })
    }
    fn copy_subtree(
        &mut self,
        source: &DocumentStorage,
        source_root: usize,
        parent: usize,
        root_name: Option<TextSpan>,
        nodes: &mut usize,
        settings: CopySettings<'_>,
    ) -> Result<(), SchemaIssue> {
        let limits = settings.limits;
        let mut source_index = source_root;
        let mut target_parent = parent;
        let mut depth = 2usize;
        let result = (|| {
            loop {
                if settings.enforce_logical_limits && depth > limits.depth.get() {
                    return Err(exceeded(SchemaResource::Depth));
                }
                *nodes = nodes
                    .checked_add(1)
                    .ok_or_else(|| exceeded(SchemaResource::Steps))?;
                if settings.enforce_logical_limits && *nodes > limits.nodes.get() {
                    return Err(exceeded(SchemaResource::Steps));
                }
                let original = *source
                    .node(source_index)
                    .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                if settings.enforce_logical_limits
                    && matches!(original.kind, ManagedKind::Array | ManagedKind::Object)
                    && original.len > limits.collection.get()
                {
                    return Err(exceeded(SchemaResource::Width));
                }
                let kind = match original.kind {
                    ManagedKind::Null => ManagedKind::Null,
                    ManagedKind::Bool(value) => ManagedKind::Bool(value),
                    ManagedKind::Number(span) => ManagedKind::Number(self.copy_span(source, span)?),
                    ManagedKind::String(span) => ManagedKind::String(self.copy_span(source, span)?),
                    ManagedKind::Array => ManagedKind::Array,
                    ManagedKind::Object => ManagedKind::Object,
                };
                let name = if source_index == source_root {
                    root_name
                } else {
                    original
                        .name
                        .map(|name| self.copy_span(source, name))
                        .transpose()?
                };
                let mut target_index = self.add(kind)?;
                self.append(target_parent, target_index, name)?;
                if let Some(first) = original.first {
                    source_index = first;
                    target_parent = target_index;
                    depth = depth
                        .checked_add(1)
                        .ok_or_else(|| exceeded(SchemaResource::Depth))?;
                    continue;
                }
                loop {
                    if source_index == source_root {
                        return Ok(());
                    }
                    let current = *source
                        .node(source_index)
                        .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                    if let Some(next) = current.next {
                        target_parent = self
                            .node(target_index)?
                            .parent
                            .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                        source_index = next;
                        break;
                    }
                    source_index = current
                        .parent
                        .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                    target_index = self
                        .node(target_index)?
                        .parent
                        .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                    depth = depth
                        .checked_sub(1)
                        .ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                }
            }
        })();
        result.map_err(|issue| {
            if settings.unpathed_logical_limits
                && matches!(
                    issue.kind(),
                    SchemaIssueKind::LimitExceeded {
                        resource: SchemaResource::Depth
                            | SchemaResource::Steps
                            | SchemaResource::Width
                    }
                )
            {
                issue
            } else {
                let (issue, refused) =
                    source.locate_subtree(source_root, source_index, issue, settings.issues);
                if refused {
                    issue
                } else if let Some(index) = settings.outer_index {
                    super::super::super::parse_context::selected_issue_at_control(
                        settings.issues,
                        issue,
                        super::super::super::IssueSegment::Index(index),
                    )
                    .0
                } else {
                    issue
                }
            }
        })
    }
    fn append(
        &mut self,
        parent: usize,
        child: usize,
        name: Option<TextSpan>,
    ) -> Result<(), SchemaIssue> {
        let previous = self.node(parent)?.last;
        {
            let node = self.node_mut(child)?;
            node.parent = Some(parent);
            node.name = name;
        }
        if let Some(previous) = previous {
            self.node_mut(previous)?.next = Some(child);
        } else {
            self.node_mut(parent)?.first = Some(child);
        }
        let node = self.node_mut(parent)?;
        node.last = Some(child);
        node.len = node
            .len
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Width))?;
        Ok(())
    }
}
impl Backend for Arena<'_> {
    type Value = usize;
    type Key = TextSpan;
    type Array = usize;
    type Object = usize;
    fn string(&mut self, token: &str) -> Result<TextSpan, StringDecodeError> {
        decode_quoted(token, |_| Ok(()))?;
        let start = self.text.len();
        decode_quoted(token, |text| {
            self.text
                .extend_from_slice(text.as_bytes())
                .map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error)))
        })?;
        Ok(TextSpan {
            start,
            end: self.text.len(),
        })
    }
    fn string_value(&mut self, value: TextSpan) -> Result<usize, SchemaIssue> {
        self.add(ManagedKind::String(value))
    }
    fn scalar(&mut self, token: &str) -> Result<usize, Option<SchemaIssue>> {
        let kind = match token {
            "null" => ManagedKind::Null,
            "true" => ManagedKind::Bool(true),
            "false" => ManagedKind::Bool(false),
            _ => {
                super::super::super::managed_number::ManagedNumber::parse(token).ok_or(None)?;
                let start = self.text.len();
                self.text
                    .extend_from_slice(token.as_bytes())
                    .map_err(|error| Some(SchemaIssue::new(SchemaIssueKind::Memory(error))))?;
                ManagedKind::Number(TextSpan {
                    start,
                    end: self.text.len(),
                })
            }
        };
        self.add(kind).map_err(Some)
    }
    fn array(&mut self) -> Result<usize, SchemaIssue> {
        self.add(ManagedKind::Array)
    }
    fn object(&mut self) -> Result<usize, SchemaIssue> {
        self.add(ManagedKind::Object)
    }
    fn array_len(&self, array: &usize) -> Result<usize, SchemaIssue> {
        Ok(self.node(*array)?.len)
    }
    fn array_push(&mut self, array: &mut usize, value: usize) -> Result<(), SchemaIssue> {
        self.append(*array, value, None)
    }
    fn object_insert(
        &mut self,
        object: &mut usize,
        key: TextSpan,
        value: usize,
        policy: DuplicateFields,
        mode: PathMode<'_>,
    ) -> Result<(), SchemaIssue> {
        let mut previous = None;
        let mut cursor = self.node(*object)?.first;
        while let Some(index) = cursor {
            let node = self.node(index)?;
            if let Some(name) = node.name
                && self.text(name)? == self.text(key)?
            {
                if policy == DuplicateFields::Reject {
                    let name = self.text(key)?;
                    return Err(match mode {
                        PathMode::Ordinary => {
                            SchemaIssue::new(SchemaIssueKind::DuplicateField(name.into()))
                        }
                        PathMode::Prepared(memory) => {
                            match super::super::super::schema_issue::format_diagnostic_text(
                                memory,
                                format_args!("{name}"),
                            ) {
                                Ok(name) => {
                                    SchemaIssue::new(SchemaIssueKind::PreparedDuplicateField(name))
                                }
                                Err(issue) => issue,
                            }
                        }
                    });
                }
                if policy == DuplicateFields::KeepLast {
                    let next = node.next;
                    {
                        let child = self.node_mut(value)?;
                        child.parent = Some(*object);
                        child.name = Some(key);
                        child.next = next;
                    }
                    if let Some(previous) = previous {
                        self.node_mut(previous)?.next = Some(value);
                    } else {
                        self.node_mut(*object)?.first = Some(value);
                    }
                    if next.is_none() {
                        self.node_mut(*object)?.last = Some(value);
                    }
                }
                return Ok(());
            }
            previous = Some(index);
            cursor = node.next;
        }
        self.append(*object, value, Some(key))
    }
    fn array_value(&mut self, array: usize) -> Result<usize, SchemaIssue> {
        Ok(array)
    }
    fn object_value(&mut self, object: usize) -> Result<usize, SchemaIssue> {
        Ok(object)
    }
    fn key_text<'a>(&'a self, key: &'a TextSpan) -> Result<&'a str, SchemaIssue> {
        self.text(*key)
    }
}
