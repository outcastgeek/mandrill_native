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
pub(crate) struct WriterBoundary {
    owner: std::sync::Arc<()>,
    root: bool,
    containers: Vec<ContainerBoundary>,
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
    /// Buffered destination retained for the original owned-byte API.
    output: Vec<u8>,
    /// Optional native destination; when present, output bytes are never collected.
    sink: Option<Box<dyn JsonWriterSink>>,
    /// Total successfully emitted bytes, independent of destination buffering.
    emitted: usize,
    identity: Option<std::sync::Arc<()>>,
    policy: JsonWriterPolicy,
    containers: Vec<Container>,
    root: bool,
    failed: bool,
}
impl JsonWriter {
    /// Create an empty writer with the supplied limits; no default resource policy is assumed.
    #[must_use]
    pub const fn new(policy: JsonWriterPolicy) -> Self {
        Self {
            output: Vec::new(),
            sink: None,
            emitted: 0,
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
    pub(crate) fn boundary(&mut self) -> Result<WriterBoundary, SchemaIssue> {
        if self.failed {
            return Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition));
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
        Ok(WriterBoundary {
            owner: std::sync::Arc::clone(owner),
            root: self.root,
            containers,
        })
    }
    /// Verify one appended value belongs to the original writer and leaves every caller-owned container intact.
    pub(crate) fn complete_value(&mut self, boundary: &WriterBoundary) -> Result<(), SchemaIssue> {
        let invalid = || SchemaIssue::new(SchemaIssueKind::InvalidDefinition);
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
        if self
            .emitted
            .checked_add(bytes.len())
            .is_none_or(|len| len > self.policy.bytes.get())
        {
            return Err(self.fail(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                resource: SchemaResource::Bytes,
            })));
        }
        if let Some(sink) = &mut self.sink {
            if let Err(issue) = sink.write(bytes) {
                return Err(self.fail(issue));
            }
        } else {
            self.output.extend_from_slice(bytes);
        }
        self.emitted += bytes.len();
        Ok(())
    }
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
    /// Start one array value at the current position, checking depth and byte limits.
    pub fn begin_array(&mut self) -> Result<(), SchemaIssue> {
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
        if !matches!(self.containers.last(), Some(Container::Array { .. })) {
            return Err(self.fail(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)));
        }
        self.containers.pop();
        self.emit(b"]")
    }
    /// Start one object value; names must be written with [`Self::property`] before their values.
    pub fn begin_object(&mut self) -> Result<(), SchemaIssue> {
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
                // JSON requires escaping U+0000 through U+001F as hex digits.
                character if character <= '\u{001f}' => {
                    let escaped = format!("\\u{:04x}", u32::from(character));
                    self.emit(escaped.as_bytes())?;
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
        let number = token
            .parse::<serde_json::Number>()
            .map_err(|_| self.fail(SchemaIssue::new(SchemaIssueKind::OutOfRange)))?;
        if number.to_string() != token {
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
    /// Consume the writer and return its UTF-8 bytes only for one complete successful root.
    ///
    /// An empty document, open container or prior failure returns `InvalidDefinition`;
    /// no partial output is returned.
    pub fn finish(self) -> Result<Vec<u8>, SchemaIssue> {
        if self.failed || !self.root || !self.containers.is_empty() || self.sink.is_some() {
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
        match self.sink.as_mut() {
            Some(sink) => sink.finish(),
            None => Err(SchemaIssue::new(SchemaIssueKind::InvalidDefinition)),
        }
    }
    pub(crate) fn poison(&mut self) {
        self.failed = true;
    }
}
