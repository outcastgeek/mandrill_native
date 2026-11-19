//! Checked incremental JSON writing to owned bytes or an explicit native sink.
//! Failure poisons this writer. Buffered output is exposed only after complete
//! validation; native bytes already delivered remain committed on later failure.
//! A caller can append a witnessed codec value with write_into.
use super::{JsonValue, SchemaIssue, SchemaIssueKind, SchemaResource};
use std::collections::BTreeSet;
use std::num::NonZeroUsize;

/// Owned destination for direct JSON bytes without a DOM or full-document buffer.
///
/// Methods are synchronous because existing direct codec witnesses are synchronous.
/// A native destination that waits for backpressure must be driven on the selected
/// blocking provider, rather than blocking an async reactor. The adapter owns that
/// scheduling, its bounded transport and its typed failure mapping. Bytes already
/// delivered cannot be retracted if a later codec operation fails.
pub trait JsonWriterSink: Send + Sync + 'static {
    /// Deliver this borrowed UTF-8 byte fragment before returning success.
    /// No reference to the fragment may escape the call; ownership conversion into
    /// a native frame is the destination's explicit transport responsibility.
    fn write(&mut self, bytes: &[u8]) -> Result<(), SchemaIssue>;

    /// Complete the finite document after structural validation succeeds.
    /// This is not response-body cleanup; the native body owner joins that separately.
    fn finish(&mut self) -> Result<(), SchemaIssue>;
}
/// Caller-supplied output bounds for an incremental JSON document.
#[derive(Clone, Copy, Debug)]
pub struct JsonWriterPolicy {
    /// Maximum total emitted UTF-8 bytes, including escapes, property names and punctuation.
    pub bytes: NonZeroUsize,
    /// Maximum value depth; a root value counts as one and each open container adds one.
    pub depth: NonZeroUsize,
}
/// Identity and compact container counters captured before a codec appends one value; avoids copying object-name sets.
pub(crate) struct OrdinaryBoundary {
    owner: std::sync::Arc<()>,
    root: bool,
    containers: Vec<ContainerBoundary>,
}
pub(crate) enum WriterBoundary {
    Ordinary(OrdinaryBoundary),
    Prepared(super::prepared_json_writer::Boundary),
}
enum ContainerBoundary {
    Array {
        identity: std::sync::Arc<()>,
        count: usize,
    },
    Object {
        identity: std::sync::Arc<()>,
        count: usize,
        pending: bool,
    },
}
enum Container {
    Array {
        count: usize,
        identity: std::sync::Arc<()>,
    },
    Object {
        names: BTreeSet<String>,
        identity: std::sync::Arc<()>,
        pending: bool,
    },
}
/// An owned, bounded incremental writer for exactly one JSON document.
///
/// Write one root value, pair every opened container with its closing method, and
/// supply a value immediately after each property name. State or resource errors
/// poison the writer: [`Self::finish`] then refuses to expose partial output.
/// [`Self::with_sink`] delivers fragments directly; its synchronous destination
/// requires explicit native scheduling and [`Self::finish_sink`] completion.
/// [`super::Codec::write_into_once`] can append one value to this same writer.
///
/// ```
/// use effect::{JsonWriter, JsonWriterPolicy};
/// use std::num::NonZeroUsize;
/// let mut writer = JsonWriter::new(JsonWriterPolicy {
///     bytes: NonZeroUsize::new(128).unwrap(),
///     depth: NonZeroUsize::new(4).unwrap(),
/// });
/// writer.begin_object()?;
/// writer.property("count")?;
/// writer.i64(3)?;
/// writer.end_object()?;
/// assert_eq!(writer.finish()?, br#"{"count":3}"#);
/// # Ok::<(), effect::SchemaIssue>(())
/// ```
pub struct JsonWriter {
    prepared: Option<super::prepared_json_writer::Owner>,
    serde_counting: bool,
    /// Buffered destination retained for the original owned-byte API.
    output: Vec<u8>,
    /// Optional native destination; when present, output bytes are never collected.
    sink: Option<Box<dyn JsonWriterSink>>,
    /// Total successfully emitted bytes, independent of destination buffering.
    emitted: usize,
    byte_limit_attempt: Option<Option<usize>>,
    identity: Option<std::sync::Arc<()>>,
    policy: JsonWriterPolicy,
    containers: Vec<Container>,
    root: bool,
    failed: bool,
}
impl JsonWriter {
    pub(crate) fn issue_memory(&self) -> Option<&super::PreparedIssueMemory> {
        self.prepared.as_ref().and_then(|owner| owner.issues.as_deref())
    }
    /// Prefix a child issue using this writer's selected diagnostic backing.
    #[doc(hidden)]
    #[must_use]
    pub fn issue_at(&self, issue: SchemaIssue, segment: super::IssueSegment) -> SchemaIssue {
        match &self.prepared {
            Some(owner) => super::parse_context::selected_issue_at(owner.issues.as_deref(), issue, segment),
            None => issue.at(segment),
        }
    }
    /// Create an empty writer with the supplied limits; no default resource policy is assumed.
    #[must_use]
    pub const fn new(policy: JsonWriterPolicy) -> Self {
        Self {
            prepared: None,
            serde_counting: false,
            output: Vec::new(),
            sink: None,
            emitted: 0,
            byte_limit_attempt: None,
            identity: None,
            policy,
            containers: Vec::new(),
            root: false,
            failed: false,
        }
    }
    /// Create a checked direct writer sending each fragment to an owned native sink.
    /// Output limits count the entire document, even when the destination drains
    /// each fragment immediately. A codec must retain its direct-writer witness;
    /// use [`Self::finish_sink`] to validate and complete this destination.
    #[must_use]
    pub fn with_sink(policy: JsonWriterPolicy, sink: impl JsonWriterSink) -> Self {
        Self {
            sink: Some(Box::new(sink)),
            ..Self::new(policy)
        }
    }
    pub(super) fn from_prepared(policy: JsonWriterPolicy, owner: super::prepared_json_writer::Owner) -> Self {
        Self { prepared: Some(owner), ..Self::new(policy) }
    }
    pub(super) fn from_serde_counting(policy: JsonWriterPolicy, owner: super::prepared_json_writer::Owner) -> Self {
        Self { prepared: Some(owner), serde_counting: true, ..Self::new(policy) }
    }
    /// Return a complete prepared buffered document with its real backing retained.
    pub fn finish_prepared(mut self) -> Result<super::PreparedJsonOutput, SchemaIssue> {
        if self.failed || !self.root || self.prepared.as_ref().is_none_or(|owner| !owner.storage.containers.is_empty() || owner.has_sink()) {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        let owner = self.prepared.take().ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        Ok(super::PreparedJsonOutput { owner })
    }
    pub(crate) fn boundary(&mut self) -> Result<WriterBoundary, SchemaIssue> {
        if self.failed {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        if self.prepared.is_some() {
            use super::prepared_json_writer::{Boundary, BoundaryFrame, Container as C};
            let result = (|| {
            let owner = self.prepared.as_mut().ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
            let token = owner.token.try_clone().map_err(|_| SchemaIssue::new(SchemaIssueKind::LimitExceeded { resource: SchemaResource::WriterBoundaries }))?;
            let mut snapshot = owner.snapshots.checkout().map_err(|_| SchemaIssue::new(SchemaIssueKind::LimitExceeded { resource: SchemaResource::WriterBoundaries }))?;
            for entry in owner.storage.containers.as_slice() {
                let frame = match entry {
                    C::Array { serial, count } => BoundaryFrame::Array { serial: *serial, count: *count },
                    C::Object { serial, count, pending, .. } => BoundaryFrame::Object { serial: *serial, count: *count, pending: *pending },
                };
                if snapshot.frames.push(frame).is_err() { snapshot.frames.clear(); return Err(SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::NoSlot))); }
            }
            Ok(WriterBoundary::Prepared(Boundary { snapshot, token, root: self.root }))
            })();
            return result.map_err(|issue| self.fail(issue));
        }
        let owner = self.identity.get_or_insert_with(|| std::sync::Arc::new(()));
        let containers = self
            .containers
            .iter()
            .map(|container| match container {
                Container::Array { count, identity } => ContainerBoundary::Array {
                    identity: std::sync::Arc::clone(identity),
                    count: *count,
                },
                Container::Object {
                    names,
                    pending,
                    identity,
                } => ContainerBoundary::Object {
                    identity: std::sync::Arc::clone(identity),
                    count: names.len(),
                    pending: *pending,
                },
            })
            .collect();
        Ok(WriterBoundary::Ordinary(OrdinaryBoundary {
            owner: std::sync::Arc::clone(owner),
            root: self.root,
            containers,
        }))
    }
    /// Verify one appended value belongs to the original writer and leaves every caller-owned container intact.
    pub(crate) fn complete_value(&mut self, boundary: &WriterBoundary) -> Result<(), SchemaIssue> {
        let invalid = || SchemaIssue::new(SchemaIssueKind::InvalidDefinition);
        if let WriterBoundary::Prepared(boundary) = boundary {
            use super::prepared_json_writer::{BoundaryFrame as B, Container as C};
            let Some(owner) = &self.prepared else { return Err(self.fail(invalid())); };
            let before = boundary.snapshot.frames.as_slice();
            let after = owner.storage.containers.as_slice();
            if self.failed || !owner.token.same_slot(&boundary.token) || before.len() != after.len() { return Err(self.fail(invalid())); }
            if before.is_empty() { if boundary.root || !self.root { return Err(self.fail(invalid())); } return Ok(()); }
            if self.root != boundary.root { return Err(self.fail(invalid())); }
            for (index, (left, right)) in before.iter().zip(after).enumerate() {
                let last = index.saturating_add(1) == before.len();
                let valid = match (left, right) {
                    (B::Array { serial: a, count: x }, C::Array { serial: b, count: y }) => a == b && if last { x.checked_add(1) == Some(*y) } else { x == y },
                    (B::Object { serial: a, count: x, pending: p }, C::Object { serial: b, count: y, pending: q, .. }) => a == b && x == y && if last { *p && !*q } else { p == q },
                    _ => false,
                };
                if !valid { return Err(self.fail(invalid())); }
            }
            return Ok(());
        }
        let WriterBoundary::Ordinary(boundary) = boundary else { return Err(self.fail(invalid())); };
        if self.prepared.is_some() { return Err(self.fail(invalid())); }
        if self.failed
            || self
                .identity
                .as_ref()
                .is_none_or(|owner| !std::sync::Arc::ptr_eq(owner, &boundary.owner))
            || boundary.containers.len() != self.containers.len()
        {
            return Err(self.fail(invalid()));
        }
        if boundary.containers.is_empty() {
            if boundary.root || !self.root {
                return Err(self.fail(invalid()));
            }
            return Ok(());
        }
        if self.root != boundary.root {
            return Err(self.fail(invalid()));
        }
        let length = boundary.containers.len();
        for (index, (before, after)) in boundary.containers.iter().zip(&self.containers).enumerate()
        {
            let last = index.saturating_add(1) == length;
            let valid = match (before, after) {
                (
                    ContainerBoundary::Array {
                        count: before,
                        identity: left,
                    },
                    Container::Array {
                        count: after,
                        identity: right,
                    },
                ) => {
                    std::sync::Arc::ptr_eq(left, right)
                        && if last {
                            before.checked_add(1) == Some(*after)
                        } else {
                            before == after
                        }
                }
                (
                    ContainerBoundary::Object {
                        count,
                        pending: before,
                        identity: left,
                    },
                    Container::Object {
                        names,
                        pending: after,
                        identity: right,
                    },
                ) => {
                    std::sync::Arc::ptr_eq(left, right)
                        && *count == names.len()
                        && if last {
                            *before && !*after
                        } else {
                            before == after
                        }
                }
                _ => false,
            };
            if !valid {
                return Err(self.fail(invalid()));
            }
        }
        Ok(())
    }
    pub(crate) const fn policy(&self) -> JsonWriterPolicy {
        self.policy
    }
    fn fail(&mut self, issue: SchemaIssue) -> SchemaIssue {
        self.failed = true;
        issue
    }
    fn emit(&mut self, bytes: &[u8]) -> Result<(), SchemaIssue> {
        if self.failed {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        let attempted = self.emitted.checked_add(bytes.len());
        if attempted.is_none_or(|len| len > self.policy.bytes.get()) {
            self.byte_limit_attempt = Some(attempted);
            return Err(self.fail(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                resource: SchemaResource::Bytes,
            })));
        }
        if let Some(owner) = &mut self.prepared {
            let result = match owner.write_sink(bytes) {
                Some(result) => result,
                None => owner.storage.output.extend_from_slice(bytes).map_err(|error| SchemaIssue::new(SchemaIssueKind::Memory(error))),
            };
            if let Err(issue) = result { return Err(self.fail(issue)); }
        } else if let Some(sink) = &mut self.sink {
            if let Err(issue) = sink.write(bytes) {
                return Err(self.fail(issue));
            }
        } else {
            self.output.extend_from_slice(bytes);
        }
        self.emitted += bytes.len();
        Ok(())
    }
    pub(crate) fn byte_limit_attempt(&self) -> Option<Option<usize>> { self.byte_limit_attempt }
    fn value(&mut self) -> Result<(), SchemaIssue> {
        if self
            .containers
            .len()
            .checked_add(1)
            .is_none_or(|depth| depth > self.policy.depth.get())
        {
            return Err(self.fail(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                resource: SchemaResource::JsonDepth,
            })));
        }
        if let Some(owner) = &mut self.prepared {
            use super::prepared_json_writer::Container as PreparedContainer;
            let depth = owner.storage.containers.len();
            if depth.checked_add(1).is_none_or(|n| n > self.policy.depth.get()) {
                return Err(self.fail(SchemaIssue::new(SchemaIssueKind::LimitExceeded { resource: SchemaResource::JsonDepth })));
            }
            let mut comma = false;
            match owner.storage.containers.as_mut_slice().last_mut() {
                Some(PreparedContainer::Array { count, .. }) => {
                    comma = *count > 0;
                    let Some(next) = count.checked_add(1) else { return Err(self.fail(SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::ArithmeticOverflow)))); };
                    *count = next;
                }
                Some(PreparedContainer::Object { pending, .. }) if *pending => *pending = false,
                Some(PreparedContainer::Object { .. }) => return Err(self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition))),
                None if self.root => return Err(self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition))),
                None => self.root = true,
            }
            if comma { self.emit(b",")?; }
            return Ok(());
        }
        match self.containers.last_mut() {
            Some(Container::Array { count, .. }) => {
                let comma = *count > 0;
                *count += 1;
                if comma {
                    self.emit(b",")?;
                }
            }
            Some(Container::Object { pending, .. }) if *pending => *pending = false,
            Some(Container::Object { .. }) => {
                return Err(self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)));
            }
            None if self.root => {
                return Err(self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)));
            }
            None => self.root = true,
        }
        Ok(())
    }
    fn prepared_begin(&mut self, object: bool) -> Result<(), SchemaIssue> {
        use super::prepared_json_writer::Container as C;
        let owner = self.prepared.as_ref().ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        if owner.storage.containers.len() == owner.storage.containers.capacity() { return Err(self.fail(SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::NoSlot)))); }
        let Some(serial) = owner.storage.serial.checked_add(1) else { return Err(self.fail(SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::ArithmeticOverflow)))); };
        self.value()?;
        self.emit(if object { b"{" } else { b"[" })?;
        let owner = self.prepared.as_mut().ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        owner.storage.serial = serial;
        let entry = if object { C::Object { serial, names_base: owner.storage.index.len(), bytes_base: owner.storage.names.len(), count: 0, pending: false } } else { C::Array { serial, count: 0 } };
        if owner.storage.containers.push(entry).is_err() { return Err(self.fail(SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::NoSlot)))); }
        Ok(())
    }
    fn prepared_end(&mut self, object: bool) -> Result<(), SchemaIssue> {
        use super::prepared_json_writer::Container as C;
        let owner = self.prepared.as_mut().ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        let entry = owner.storage.containers.as_slice().last().copied();
        match (object, entry) {
            (false, Some(C::Array { .. })) => { owner.storage.containers.pop(); },
            (true, Some(C::Object { names_base, bytes_base, pending: false, .. })) => {
                owner.storage.containers.pop();
                while owner.storage.index.len() > names_base { owner.storage.index.pop(); }
                owner.storage.names.truncate(bytes_base);
            },
            _ => return Err(self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition))),
        }
        self.emit(if object { b"}" } else { b"]" })
    }
    fn prepared_property(&mut self, name: &str) -> Result<(), SchemaIssue> {
        use super::prepared_json_writer::{Container as C, NameSpan};
        let owner = self.prepared.as_mut().ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
        let Some(C::Object { names_base, count, pending: false, .. }) = owner.storage.containers.as_slice().last().copied() else { return Err(self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition))); };
        let Some(next_count) = count.checked_add(1) else { return Err(self.fail(SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::ArithmeticOverflow)))); };
        let duplicate = owner.storage.index.as_slice().iter().skip(names_base).any(|span| owner.storage.names.as_slice().get(span.start..span.end) == Some(name.as_bytes()));
        if duplicate { let issue = super::schema_issue::prepared_duplicate_field(owner.issues.as_deref(), name); return Err(self.fail(issue)); }
        if owner.storage.index.len() == owner.storage.index.capacity() { return Err(self.fail(SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::NoSlot)))); }
        let start = owner.storage.names.len();
        if let Err(error) = owner.storage.names.extend_from_slice(name.as_bytes()) { return Err(self.fail(SchemaIssue::new(SchemaIssueKind::Memory(error)))); }
        let end = owner.storage.names.len();
        if owner.storage.index.push(NameSpan { start, end }).is_err() { owner.storage.names.truncate(start); return Err(self.fail(SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::NoSlot)))); }
        if let Some(C::Object { count, .. }) = owner.storage.containers.as_mut_slice().last_mut() { *count = next_count; }
        if count > 0 { self.emit(b",")?; }
        self.quoted(name)?;
        self.emit(b":")?;
        if let Some(owner) = &mut self.prepared && let Some(C::Object { pending, .. }) = owner.storage.containers.as_mut_slice().last_mut() { *pending = true; }
        Ok(())
    }
    /// Start one array value at the current position, checking depth and byte limits.
    pub fn begin_array(&mut self) -> Result<(), SchemaIssue> {
        if self.prepared.is_some() { return self.prepared_begin(false); }
        self.value()?;
        self.emit(b"[")?;
        self.containers.push(Container::Array {
            count: 0,
            identity: std::sync::Arc::new(()),
        });
        Ok(())
    }
    /// Close the current array. A different current container is an invalid-definition error.
    pub fn end_array(&mut self) -> Result<(), SchemaIssue> {
        if self.prepared.is_some() { return self.prepared_end(false); }
        if !matches!(self.containers.last(), Some(Container::Array { .. })) {
            return Err(self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)));
        }
        self.containers.pop();
        self.emit(b"]")
    }
    /// Start one object value; names must be written with [`Self::property`] before their values.
    pub fn begin_object(&mut self) -> Result<(), SchemaIssue> {
        if self.prepared.is_some() { return self.prepared_begin(true); }
        self.value()?;
        self.emit(b"{")?;
        self.containers.push(Container::Object {
            names: BTreeSet::new(),
            identity: std::sync::Arc::new(()),
            pending: false,
        });
        Ok(())
    }
    /// Close the current object; an unfilled property or non-object current container poisons the writer.
    pub fn end_object(&mut self) -> Result<(), SchemaIssue> {
        if self.prepared.is_some() { return self.prepared_end(true); }
        if !matches!(
            self.containers.last(),
            Some(Container::Object { pending: false, .. })
        ) {
            return Err(self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)));
        }
        self.containers.pop();
        self.emit(b"}")
    }
    /// Write a unique property name in the current object and reserve its next value.
    ///
    /// Rejects duplicates, use outside an object, or another name while a value is
    /// pending. Names are escaped as JSON strings and count toward the byte limit.
    pub fn property(&mut self, name: &str) -> Result<(), SchemaIssue> {
        if self.prepared.is_some() { return self.prepared_property(name); }
        let comma = match self.containers.last_mut() {
            Some(Container::Object {
                names,
                pending: false,
                ..
            }) => {
                let comma = !names.is_empty();
                if !names.insert(name.to_owned()) {
                    return Err(self.fail(SchemaIssue::new(SchemaIssueKind::DuplicateField(
                        name.into(),
                    ))));
                }
                comma
            }
            _ => return Err(self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition))),
        };
        if comma {
            self.emit(b",")?;
        }
        self.quoted(name)?;
        self.emit(b":")?;
        if let Some(Container::Object { pending, .. }) = self.containers.last_mut() {
            *pending = true;
        }
        Ok(())
    }
    fn quoted(&mut self, value: &str) -> Result<(), SchemaIssue> {
        self.emit(b"\"")?;
        for character in value.chars() {
            match character {
                '"' => self.emit(b"\\\"")?,
                '\\' => self.emit(b"\\\\")?,
                '\n' => self.emit(b"\\n")?,
                '\r' => self.emit(b"\\r")?,
                '\t' => self.emit(b"\\t")?,
                '\u{0008}' if self.serde_counting => self.emit(b"\\b")?,
                '\u{000c}' if self.serde_counting => self.emit(b"\\f")?,
                // JSON requires escaping U+0000 through U+001F as hex digits.
                character if character <= '\u{001f}' => {
                    let mut escaped = *b"\\u0000";
                    let code = u32::from(character);
                    let hex = b"0123456789abcdef";
                    if let (Some([target_high, target_low]), Some(high), Some(low)) = (escaped.get_mut(4..), hex.get(usize::try_from(code >> 4).unwrap_or_default()), hex.get(usize::try_from(code & 15).unwrap_or_default())) {
                        *target_high = *high;
                        *target_low = *low;
                    }
                    self.emit(&escaped)?;
                }
                character => {
                    let mut utf8 = [0; std::mem::size_of::<char>()];
                    self.emit(character.encode_utf8(&mut utf8).as_bytes())?;
                }
            }
        }
        self.emit(b"\"")
    }
    /// Write one JSON string, escaping control characters and counting emitted UTF-8 bytes.
    pub fn string(&mut self, value: &str) -> Result<(), SchemaIssue> {
        self.value()?;
        self.quoted(value)
    }
    /// Write one null value at the current root, array slot or pending object property.
    pub fn null(&mut self) -> Result<(), SchemaIssue> {
        self.value()?;
        self.emit(b"null")
    }
    /// Write one boolean value, subject to the writer state and output bounds.
    pub fn boolean(&mut self, value: bool) -> Result<(), SchemaIssue> {
        self.value()?;
        self.emit(if value { b"true" } else { b"false" })
    }
    /// Write a signed integer losslessly, including the minimum i64 value.
    pub fn i64(&mut self, value: i64) -> Result<(), SchemaIssue> {
        self.value()?;
        let mut buffer = std::fmt::NumBuffer::new();
        self.emit(value.format_into(&mut buffer).as_bytes())
    }
    /// Write an unsigned integer losslessly without converting through floating point.
    pub fn u64(&mut self, value: u64) -> Result<(), SchemaIssue> {
        self.value()?;
        let mut buffer = std::fmt::NumBuffer::new();
        self.emit(value.format_into(&mut buffer).as_bytes())
    }
    /// Write an already normalized JSON number token.
    ///
    /// The token must parse as a JSON number and round-trip to the exact same spelling;
    /// otherwise returns [`SchemaIssueKind::OutOfRange`] and poisons the writer.
    pub fn number(&mut self, token: &str) -> Result<(), SchemaIssue> {
        if self.prepared.is_some() {
            if !super::managed_number::ManagedNumber::parse(token).is_some_and(|number| number.is_normalized()) {
                return Err(self.fail(SchemaIssue::new(SchemaIssueKind::OutOfRange)));
            }
            self.value()?;
            return self.emit(token.as_bytes());
        }
        let number = token
            .parse::<serde_json::Number>()
            .map_err(|_| self.fail(SchemaIssue::new(SchemaIssueKind::OutOfRange)))?;
        struct Compare<'a> { target: &'a [u8], offset: usize }
        impl std::fmt::Write for Compare<'_> {
            fn write_str(&mut self, text: &str) -> std::fmt::Result {
                let end = self.offset.checked_add(text.len()).ok_or(std::fmt::Error)?;
                if self.target.get(self.offset..end) != Some(text.as_bytes()) { return Err(std::fmt::Error); }
                self.offset = end;
                Ok(())
            }
        }
        let mut comparison = Compare { target: token.as_bytes(), offset: 0 };
        if std::fmt::write(&mut comparison, format_args!("{number}")).is_err() || comparison.offset != token.len() {
            return Err(self.fail(SchemaIssue::new(SchemaIssueKind::OutOfRange)));
        }
        self.value()?;
        self.emit(token.as_bytes())
    }
    /// Append one existing [`JsonValue`] tree iteratively, using the same checked operations.
    ///
    /// This explicit tree route needs no direct-writer witness. It borrows the tree
    /// and enforces this writer's byte, depth and container-state bounds.
    pub fn tree(&mut self, value: &JsonValue) -> Result<(), SchemaIssue> {
        if self.prepared.is_some() {
            let result = self.prepared_tree(value);
            if let Some(owner) = &mut self.prepared { owner.storage.tree.clear(); }
            return result;
        }
        enum Frame<'a> {
            Value(&'a JsonValue),
            Property(&'a str),
            EndArray,
            EndObject,
        }
        let mut frames = vec![Frame::Value(value)];
        while let Some(frame) = frames.pop() {
            match frame {
                Frame::Property(name) => self.property(name)?,
                Frame::EndArray => self.end_array()?,
                Frame::EndObject => self.end_object()?,
                Frame::Value(JsonValue::Null) => self.null()?,
                Frame::Value(JsonValue::Bool(value)) => self.boolean(*value)?,
                Frame::Value(JsonValue::String(value)) => self.string(value)?,
                Frame::Value(JsonValue::Number(number)) => {
                    if let Some(value) = number.as_i64() {
                        self.i64(value)?;
                    } else if let Some(value) = number.as_u64() {
                        self.u64(value)?;
                    } else {
                        self.number(&number.to_string())?;
                    }
                }
                Frame::Value(JsonValue::Array(items)) => {
                    self.begin_array()?;
                    frames.push(Frame::EndArray);
                    for value in items.iter().rev() {
                        frames.push(Frame::Value(value));
                    }
                }
                Frame::Value(JsonValue::Object(fields)) => {
                    self.begin_object()?;
                    frames.push(Frame::EndObject);
                    for (name, value) in fields.iter().rev() {
                        frames.push(Frame::Value(value));
                        frames.push(Frame::Property(name));
                    }
                }
            }
        }
        Ok(())
    }
    fn prepared_tree(&mut self, root: &JsonValue) -> Result<(), SchemaIssue> {
        use super::prepared_json_writer::TreeFrame;
        fn locate<'a>(root: &'a JsonValue, path: &[TreeFrame]) -> Option<&'a JsonValue> {
            let mut value = root;
            for frame in path {
                let index = frame.next.checked_sub(1)?;
                value = match value {
                    JsonValue::Array(items) => items.get(index)?,
                    JsonValue::Object(fields) => fields.iter().nth(index)?.1,
                    JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => return None,
                };
            }
            Some(value)
        }
        if let Some(owner) = &mut self.prepared { owner.storage.tree.clear(); }
        let mut current = Some(root);
        loop {
            if let Some(value) = current.take() {
                match value {
                    JsonValue::Null => self.null()?,
                    JsonValue::Bool(value) => self.boolean(*value)?,
                    JsonValue::String(value) => self.string(value)?,
                    JsonValue::Number(number) => {
                        if let Some(value) = number.as_i64() { self.i64(value)?; }
                        else if let Some(value) = number.as_u64() { self.u64(value)?; }
                        else { self.value()?; self.emit_number_display(number)?; }
                    }
                    JsonValue::Array(_) | JsonValue::Object(_) => {
                        let owner = self.prepared.as_ref().ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                        if owner.storage.tree.len() == owner.storage.tree.capacity() { return Err(self.fail(SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::NoSlot)))); }
                        if matches!(value, JsonValue::Array(_)) { self.begin_array()?; } else { self.begin_object()?; }
                        let owner = self.prepared.as_mut().ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                        if owner.storage.tree.push(TreeFrame { next: 0 }).is_err() { return Err(self.fail(SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::NoSlot)))); }
                    }
                }
            }
            let (next, name, closing) = {
                let owner = self.prepared.as_ref().ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                let Some((frame, path)) = owner.storage.tree.as_slice().split_last() else { return Ok(()); };
                let parent = locate(root, path).ok_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition))?;
                match parent {
                    JsonValue::Array(items) => match items.get(frame.next) { Some(value) => (Some(value), None, None), None => (None, None, Some(false)) },
                    JsonValue::Object(fields) => match fields.iter().nth(frame.next) { Some((name, value)) => (Some(value), Some(name.as_str()), None), None => (None, None, Some(true)) },
                    JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => return Err(self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition))),
                }
            };
            if let Some(object) = closing {
                if let Some(owner) = &mut self.prepared { owner.storage.tree.pop(); }
                if object { self.end_object()?; } else { self.end_array()?; }
            } else {
                if let Some(owner) = &mut self.prepared && let Some(frame) = owner.storage.tree.as_mut_slice().last_mut() {
                    let Some(next_index) = frame.next.checked_add(1) else { return Err(self.fail(SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::ArithmeticOverflow)))); };
                    frame.next = next_index;
                }
                if let Some(name) = name { self.property(name)?; }
                current = next;
            }
        }
    }
    pub(crate) fn managed_number(&mut self, token: &str) -> Result<(), SchemaIssue> {
        let number = super::managed_number::ManagedNumber::parse(token)
            .ok_or_else(|| self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)))?;
        let (prefix, exponent, sign, suffix) = number.pieces();
        self.value()?;
        for part in [prefix, exponent, sign, suffix] {
            if !part.is_empty() { self.emit(part.as_bytes())?; }
        }
        Ok(())
    }
    pub(crate) fn finite_f64(&mut self, value: f64) -> Result<(), SchemaIssue> {
        if !value.is_finite() { return Err(self.fail(SchemaIssue::new(SchemaIssueKind::NonFinite))); }
        // Pinned zmij 1.0.23 uses a 24-byte buffer for every finite f64.
        let mut storage = [0u8; 24];
        let used = {
            let mut remaining = storage.as_mut_slice();
            serde_json::to_writer(&mut remaining, &value)
                .map_err(|_| self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)))?;
            24 - remaining.len()
        };
        let bytes = storage.get(..used).ok_or_else(|| self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)))?;
        self.value()?;
        self.emit(bytes)
    }
    fn emit_number_display(&mut self, number: &serde_json::Number) -> Result<(), SchemaIssue> {
        struct Emit<'a> { writer: &'a mut JsonWriter, issue: Option<SchemaIssue> }
        impl std::fmt::Write for Emit<'_> {
            fn write_str(&mut self, text: &str) -> std::fmt::Result {
                match self.writer.emit(text.as_bytes()) { Ok(()) => Ok(()), Err(issue) => { self.issue = Some(issue); Err(std::fmt::Error) } }
            }
        }
        let mut output = Emit { writer: self, issue: None };
        if std::fmt::write(&mut output, format_args!("{number}")).is_err() {
            return Err(output.issue.unwrap_or_else(|| SchemaIssue::new(SchemaIssueKind::InvalidDefinition)));
        }
        Ok(())
    }
    /// Consume the writer and return its UTF-8 bytes only for one complete successful root.
    ///
    /// An empty document, open container or prior failure returns `InvalidDefinition`;
    /// no partial output is returned.
    pub fn finish(self) -> Result<Vec<u8>, SchemaIssue> {
        if self.prepared.is_some() || self.failed || !self.root || !self.containers.is_empty() || self.sink.is_some() {
            Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition))
        } else {
            Ok(self.output)
        }
    }
    /// Consume a native-sink writer after exactly one complete successful root.
    /// Structural or earlier sink failure prevents the destination's finish call;
    /// already delivered bytes remain committed and the native owner reports or
    /// terminates its response according to the selected server policy.
    pub fn finish_sink(mut self) -> Result<(), SchemaIssue> {
        if self.failed || !self.root || !self.containers.is_empty() {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
        }
        if let Some(owner) = &mut self.prepared {
            if !owner.storage.containers.is_empty() { return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)); }
            return owner.finish_sink().unwrap_or_else(|| Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)));
        }
        match self.sink.as_mut() {
            Some(sink) => sink.finish(),
            None => Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)),
        }
    }
    pub(crate) fn poison(&mut self) {
        self.failed = true;
    }
}
