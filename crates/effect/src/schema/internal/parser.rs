//! Bounded exact JSON parsing and structural checks.
use super::super::{
    DuplicateFields, JsonValue, ParseLimits, ParsePolicy, SchemaErrorMode, SchemaIssue,
    SchemaIssueKind, SchemaResource, UnknownFields,
};
use serde::Serialize;
use std::io::{self, Write};
use std::sync::Arc;

const JSON_SYNTAX_AT_BYTE: &str = "invalid JSON at byte ";

enum Container<A, O, K> {
    Array {
        values: A,
        need_value: bool,
        may_end: bool,
    },
    Object {
        fields: O,
        key: Option<K>,
        need_key: bool,
        may_end: bool,
        count: usize,
    },
}
struct Tokens<'a, 'b> {
    text: &'a str,
    offset: usize,
    mode: PathMode<'b>,
}
impl<'a> Tokens<'a, '_> {
    fn whitespace(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.offset)
            .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.offset += 1;
        }
    }
    fn byte(&mut self) -> Option<u8> {
        self.whitespace();
        self.text.as_bytes().get(self.offset).copied()
    }
    fn syntax(&self) -> SchemaIssue {
        match self.mode {
            PathMode::Ordinary => SchemaIssue::new(SchemaIssueKind::JsonSyntax {
                detail: Arc::from(format!("{JSON_SYNTAX_AT_BYTE}{}", self.offset)),
            }),
            PathMode::Prepared(memory) => match super::super::schema_issue::format_diagnostic_text(
                memory,
                format_args!("{JSON_SYNTAX_AT_BYTE}{}", self.offset),
            ) {
                Ok(detail) => SchemaIssue::new(SchemaIssueKind::PreparedJsonSyntax { detail }),
                Err(issue) => issue,
            },
        }
    }
    fn string_token(&mut self) -> Result<&'a str, SchemaIssue> {
        let start = self.offset;
        self.offset += 1;
        let mut escaped = false;
        while let Some(byte) = self.text.as_bytes().get(self.offset).copied() {
            self.offset += 1;
            if !escaped && byte == b'"' {
                return self
                    .text
                    .get(start..self.offset)
                    .ok_or_else(|| self.syntax());
            }
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            }
        }
        Err(self.syntax())
    }
    fn string<B: Backend>(&mut self, backend: &mut B) -> Result<B::Key, SchemaIssue> {
        let token = self.string_token()?;
        backend.string(token).map_err(|error| match error {
            StringDecodeError::Syntax => self.syntax(),
            StringDecodeError::Sink(issue) => issue,
        })
    }
    fn scalar<B: Backend>(&mut self, backend: &mut B) -> Result<B::Value, SchemaIssue> {
        if self.byte() == Some(b'"') {
            let value = self.string(backend)?;
            return backend.string_value(value);
        }
        let start = self.offset;
        while self
            .text
            .as_bytes()
            .get(self.offset)
            .is_some_and(|byte| !matches!(byte, b' ' | b'\n' | b'\r' | b'\t' | b',' | b']' | b'}'))
        {
            self.offset += 1;
        }
        let token = self
            .text
            .get(start..self.offset)
            .ok_or_else(|| self.syntax())?;
        backend
            .scalar(token)
            .map_err(|error| error.unwrap_or_else(|| self.syntax()))
    }
}

enum StringDecodeError {
    Syntax,
    Sink(SchemaIssue),
}
// Both ordinary and prepared backends use this decoder after the same complete
// token scan, so syntax errors keep the scanner's end-of-token offset.
fn decode_quoted(
    token: &str,
    mut emit: impl FnMut(&str) -> Result<(), SchemaIssue>,
) -> Result<(), StringDecodeError> {
    fn hex(chars: &mut std::str::Chars<'_>) -> Result<u16, StringDecodeError> {
        let mut value = 0u16;
        for _ in 0..4 {
            let digit = match chars.next() {
                Some(c @ '0'..='9') => u32::from(c) - u32::from('0'),
                Some(c @ 'a'..='f') => u32::from(c) - u32::from('a') + 10,
                Some(c @ 'A'..='F') => u32::from(c) - u32::from('A') + 10,
                _ => return Err(StringDecodeError::Syntax),
            };
            let digit = u16::try_from(digit).map_err(|_| StringDecodeError::Syntax)?;
            value = (value << 4) | digit;
        }
        Ok(value)
    }
    let inner = token
        .get(1..token.len().saturating_sub(1))
        .ok_or(StringDecodeError::Syntax)?;
    let mut chars = inner.chars();
    while let Some(character) = chars.next() {
        let decoded = match character {
            '\\' => match chars.next() {
                Some('"') => '"',
                Some('\\') => '\\',
                Some('/') => '/',
                Some('b') => '\u{0008}',
                Some('f') => '\u{000c}',
                Some('n') => '\n',
                Some('r') => '\r',
                Some('t') => '\t',
                Some('u') => {
                    let first = hex(&mut chars)?;
                    let code = if (0xd800..=0xdbff).contains(&first) {
                        if chars.next() != Some('\\') || chars.next() != Some('u') {
                            return Err(StringDecodeError::Syntax);
                        }
                        let second = hex(&mut chars)?;
                        if !(0xdc00..=0xdfff).contains(&second) {
                            return Err(StringDecodeError::Syntax);
                        }
                        0x10000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
                    } else {
                        u32::from(first)
                    };
                    char::from_u32(code).ok_or(StringDecodeError::Syntax)?
                }
                _ => return Err(StringDecodeError::Syntax),
            },
            value if value <= '\u{001f}' => return Err(StringDecodeError::Syntax),
            value => value,
        };
        let mut bytes = [0u8; 4];
        emit(decoded.encode_utf8(&mut bytes)).map_err(StringDecodeError::Sink)?;
    }
    Ok(())
}
trait ParseStack<T> {
    fn push(&mut self, value: T) -> Result<(), SchemaIssue>;
    fn pop(&mut self) -> Option<T>;
    fn last_mut(&mut self) -> Option<&mut T>;
    fn len(&self) -> usize;
    fn as_slice(&self) -> &[T];
}
impl<T> ParseStack<T> for Vec<T> {
    fn push(&mut self, value: T) -> Result<(), SchemaIssue> {
        Vec::push(self, value);
        Ok(())
    }
    fn pop(&mut self) -> Option<T> {
        Vec::pop(self)
    }
    fn last_mut(&mut self) -> Option<&mut T> {
        self.as_mut_slice().last_mut()
    }
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn as_slice(&self) -> &[T] {
        Vec::as_slice(self)
    }
}
impl<T> ParseStack<T> for &mut crate::memory::FixedVector<'static, T> {
    fn push(&mut self, value: T) -> Result<(), SchemaIssue> {
        crate::memory::FixedVector::push(self, value).map_err(|_| {
            SchemaIssue::new(SchemaIssueKind::Memory(crate::memory::MemoryError::NoSlot))
        })
    }
    fn pop(&mut self) -> Option<T> {
        crate::memory::FixedVector::pop(self)
    }
    fn last_mut(&mut self) -> Option<&mut T> {
        self.as_mut_slice().last_mut()
    }
    fn len(&self) -> usize {
        crate::memory::FixedVector::len(self)
    }
    fn as_slice(&self) -> &[T] {
        crate::memory::FixedVector::as_slice(self)
    }
}
trait Backend {
    type Value;
    type Key;
    type Array;
    type Object;
    fn string(&mut self, token: &str) -> Result<Self::Key, StringDecodeError>;
    fn string_value(&mut self, value: Self::Key) -> Result<Self::Value, SchemaIssue>;
    fn scalar(&mut self, token: &str) -> Result<Self::Value, Option<SchemaIssue>>;
    fn array(&mut self) -> Result<Self::Array, SchemaIssue>;
    fn object(&mut self) -> Result<Self::Object, SchemaIssue>;
    fn array_len(&self, array: &Self::Array) -> Result<usize, SchemaIssue>;
    fn array_push(
        &mut self,
        array: &mut Self::Array,
        value: Self::Value,
    ) -> Result<(), SchemaIssue>;
    fn object_insert(
        &mut self,
        object: &mut Self::Object,
        key: Self::Key,
        value: Self::Value,
        policy: DuplicateFields,
        mode: PathMode<'_>,
    ) -> Result<(), SchemaIssue>;
    fn array_value(&mut self, array: Self::Array) -> Result<Self::Value, SchemaIssue>;
    fn object_value(&mut self, object: Self::Object) -> Result<Self::Value, SchemaIssue>;
    fn key_text<'a>(&'a self, key: &'a Self::Key) -> Result<&'a str, SchemaIssue>;
}
struct Ordinary;
impl Backend for Ordinary {
    type Value = JsonValue;
    type Key = String;
    type Array = Vec<JsonValue>;
    type Object = serde_json::Map<String, JsonValue>;
    fn string(&mut self, token: &str) -> Result<String, StringDecodeError> {
        let mut output = String::new();
        decode_quoted(token, |text| {
            output.push_str(text);
            Ok(())
        })?;
        Ok(output)
    }
    fn string_value(&mut self, value: String) -> Result<JsonValue, SchemaIssue> {
        Ok(JsonValue::String(value))
    }
    fn scalar(&mut self, token: &str) -> Result<JsonValue, Option<SchemaIssue>> {
        match token {
            "null" => Ok(JsonValue::Null),
            "true" => Ok(JsonValue::Bool(true)),
            "false" => Ok(JsonValue::Bool(false)),
            _ => token
                .parse::<serde_json::Number>()
                .map(JsonValue::Number)
                .map_err(|_| None),
        }
    }
    fn array(&mut self) -> Result<Self::Array, SchemaIssue> {
        Ok(Vec::new())
    }
    fn object(&mut self) -> Result<Self::Object, SchemaIssue> {
        Ok(serde_json::Map::new())
    }
    fn array_len(&self, array: &Self::Array) -> Result<usize, SchemaIssue> {
        Ok(array.len())
    }
    fn array_push(&mut self, array: &mut Self::Array, value: JsonValue) -> Result<(), SchemaIssue> {
        array.push(value);
        Ok(())
    }
    fn object_insert(
        &mut self,
        object: &mut Self::Object,
        key: String,
        value: JsonValue,
        policy: DuplicateFields,
        _mode: PathMode<'_>,
    ) -> Result<(), SchemaIssue> {
        let duplicate = object.contains_key(&key);
        if duplicate && policy == DuplicateFields::Reject {
            return Err(SchemaIssue::new(SchemaIssueKind::DuplicateField(
                key.into(),
            )));
        }
        if !duplicate || policy == DuplicateFields::KeepLast {
            object.insert(key, value);
        }
        Ok(())
    }
    fn array_value(&mut self, array: Self::Array) -> Result<JsonValue, SchemaIssue> {
        Ok(JsonValue::Array(array))
    }
    fn object_value(&mut self, object: Self::Object) -> Result<JsonValue, SchemaIssue> {
        Ok(JsonValue::Object(object))
    }
    fn key_text<'a>(&'a self, key: &'a String) -> Result<&'a str, SchemaIssue> {
        Ok(key)
    }
}
pub(crate) fn parse_json(text: &str, policy: ParsePolicy) -> Result<JsonValue, SchemaIssue> {
    let value = parse_machine(text, policy, &mut Ordinary, Vec::new(), PathMode::Ordinary)?;
    check_limits(&value, &policy.limits)?;
    Ok(value)
}
#[derive(Clone, Copy)]
enum PathMode<'a> {
    Ordinary,
    Prepared(Option<&'a super::super::PreparedIssueMemory>),
}
impl PathMode<'_> {
    fn field(self, name: &str) -> Result<super::super::IssueSegment, SchemaIssue> {
        match self {
            Self::Ordinary => Ok(super::super::IssueSegment::Field(name.into())),
            Self::Prepared(memory) => {
                super::super::schema_issue::format_diagnostic_text(memory, format_args!("{name}"))
                    .map(super::super::IssueSegment::PreparedField)
            }
        }
    }
}
fn locate<B: Backend>(
    mut issue: SchemaIssue,
    stack: &[Container<B::Array, B::Object, B::Key>],
    backend: &B,
    mode: PathMode<'_>,
) -> SchemaIssue {
    for container in stack.iter().rev() {
        let segment = match container {
            Container::Array { values, .. } => match backend.array_len(values) {
                Ok(len) => Some(super::super::IssueSegment::Index(len)),
                Err(error) => {
                    issue = error;
                    continue;
                }
            },
            Container::Object {
                key: Some(name), ..
            } => match backend.key_text(name) {
                Ok(name) => match mode.field(name) {
                    Ok(segment) => Some(segment),
                    Err(error) => {
                        return if super::super::prepared_json_validation::physical_cause(&issue)
                            .is_some()
                        {
                            issue
                        } else {
                            error
                        };
                    }
                },
                Err(error) => {
                    issue = error;
                    continue;
                }
            },
            Container::Object { key: None, .. } => None,
        };
        if let Some(segment) = segment {
            match mode {
                PathMode::Ordinary => issue = issue.at(segment),
                PathMode::Prepared(memory) => {
                    let (next, refused) = super::super::parse_context::selected_issue_at_control(
                        memory, issue, segment,
                    );
                    issue = next;
                    if refused {
                        break;
                    }
                }
            }
        }
    }
    issue
}
fn parse_machine<B: Backend, S: ParseStack<Container<B::Array, B::Object, B::Key>>>(
    text: &str,
    policy: ParsePolicy,
    backend: &mut B,
    mut stack: S,
    mode: PathMode<'_>,
) -> Result<B::Value, SchemaIssue> {
    if text.len() > policy.limits.bytes.get() {
        return Err(exceeded(SchemaResource::Bytes));
    }
    let mut tokens = Tokens {
        text,
        offset: 0,
        mode,
    };
    let mut nodes = 0usize;
    let mut completed = None;
    loop {
        if let Some(value) = completed.take() {
            match stack.last_mut() {
                Some(Container::Array {
                    values,
                    need_value,
                    may_end,
                }) => {
                    if !*need_value {
                        return Err(tokens.syntax());
                    }
                    if backend.array_len(values)? == policy.limits.collection.get() {
                        return Err(exceeded(SchemaResource::Width));
                    }
                    backend.array_push(values, value)?;
                    *need_value = false;
                    *may_end = true;
                }
                Some(Container::Object {
                    fields,
                    key,
                    need_key,
                    may_end,
                    count,
                }) => {
                    let name = key.take().ok_or_else(|| tokens.syntax())?;
                    *count = count
                        .checked_add(1)
                        .ok_or_else(|| exceeded(SchemaResource::Width))?;
                    if *count > policy.limits.collection.get() {
                        return Err(exceeded(SchemaResource::Width));
                    }
                    backend.object_insert(fields, name, value, policy.duplicate_fields, mode)?;
                    *need_key = false;
                    *may_end = true;
                }
                None => {
                    if tokens.byte().is_some() {
                        return Err(tokens.syntax());
                    }
                    return Ok(value);
                }
            }
        }
        let byte = tokens.byte().ok_or_else(|| tokens.syntax())?;
        match stack.last_mut() {
            Some(Container::Array {
                need_value: false, ..
            }) => {
                if byte == b']' {
                    tokens.offset += 1;
                    if let Some(Container::Array { values, .. }) = stack.pop() {
                        completed = Some(backend.array_value(values)?);
                    }
                    continue;
                }
                if byte != b',' {
                    return Err(tokens.syntax());
                }
                tokens.offset += 1;
                if let Some(Container::Array {
                    need_value,
                    may_end,
                    ..
                }) = stack.last_mut()
                {
                    *need_value = true;
                    *may_end = false;
                }
                continue;
            }
            Some(Container::Array {
                need_value: true,
                may_end: true,
                ..
            }) if byte == b']' => {
                tokens.offset += 1;
                if let Some(Container::Array { values, .. }) = stack.pop() {
                    completed = Some(backend.array_value(values)?);
                }
                continue;
            }
            Some(Container::Object {
                key: None,
                need_key: false,
                ..
            }) => {
                if byte == b'}' {
                    tokens.offset += 1;
                    if let Some(Container::Object { fields, .. }) = stack.pop() {
                        completed = Some(backend.object_value(fields)?);
                    }
                    continue;
                }
                if byte != b',' {
                    return Err(tokens.syntax());
                }
                tokens.offset += 1;
                if let Some(Container::Object {
                    need_key, may_end, ..
                }) = stack.last_mut()
                {
                    *need_key = true;
                    *may_end = false;
                }
                continue;
            }
            Some(Container::Object {
                key: None,
                need_key: true,
                may_end: true,
                ..
            }) if byte == b'}' => {
                tokens.offset += 1;
                if let Some(Container::Object { fields, .. }) = stack.pop() {
                    completed = Some(backend.object_value(fields)?);
                }
                continue;
            }
            Some(Container::Object {
                key: None,
                need_key: true,
                ..
            }) => {
                if byte != b'"' {
                    return Err(tokens.syntax());
                }
                let name = tokens.string(backend)?;
                if tokens.byte() != Some(b':') {
                    return Err(tokens.syntax());
                }
                tokens.offset += 1;
                if let Some(Container::Object { key, .. }) = stack.last_mut() {
                    *key = Some(name);
                }
                continue;
            }
            Some(Container::Array { .. } | Container::Object { .. }) | None => {}
        }
        let depth = stack
            .len()
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Depth))?;
        if depth > policy.limits.depth.get() {
            return Err(locate(
                exceeded(SchemaResource::Depth),
                stack.as_slice(),
                backend,
                mode,
            ));
        }
        nodes = nodes
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Steps))?;
        if nodes > policy.limits.nodes.get() {
            return Err(locate(
                exceeded(SchemaResource::Steps),
                stack.as_slice(),
                backend,
                mode,
            ));
        }
        match byte {
            b'[' => {
                tokens.offset += 1;
                let values = backend.array()?;
                stack.push(Container::Array {
                    values,
                    need_value: true,
                    may_end: true,
                })?;
            }
            b'{' => {
                tokens.offset += 1;
                let fields = backend.object()?;
                stack.push(Container::Object {
                    fields,
                    key: None,
                    need_key: true,
                    may_end: true,
                    count: 0,
                })?;
            }
            _ => completed = Some(tokens.scalar(backend)?),
        }
    }
}

pub(crate) const fn strict_wire_policy(limits: ParseLimits) -> ParsePolicy {
    ParsePolicy {
        limits,
        errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    }
}

/// # Parameters
/// - `input` (`&[u8]`): The input value or factory consumed by this operation.
/// - `policy` (`ParsePolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Result<JsonValue, SchemaIssue>`.
pub(crate) fn parse_json_bytes(
    input: &[u8],
    policy: ParsePolicy,
) -> Result<JsonValue, SchemaIssue> {
    if input.len() > policy.limits.bytes.get() {
        return Err(exceeded(SchemaResource::Bytes));
    }
    let text = std::str::from_utf8(input).map_err(|error| {
        SchemaIssue::new(SchemaIssueKind::JsonSyntax {
            detail: Arc::from(error.to_string()),
        })
    })?;
    parse_json(text, policy)
}

/// Typed bounded writer used by schema internal parser.
struct BoundedWriter {
    count: usize,
    limit: usize,
    exceeded: bool,
}

impl Write for BoundedWriter {
    ///
    /// # Parameters
    /// - `bytes` (`&[u8]`): Encoded bytes to validate, decode, write, or transfer.
    ///
    /// # Returns
    /// Type: `io::Result<usize>`.
    /// Returns `io::Result<usize>`.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self.count.checked_add(bytes.len()) {
            Some(count) if count <= self.limit => {
                self.count = count;
                Ok(bytes.len())
            }
            _ => {
                self.exceeded = true;
                Err(io::ErrorKind::Other.into())
            }
        }
    }

    ///
    /// # Returns
    /// Type: `io::Result<()>`.
    /// Returns `io::Result<()>`.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

///
/// # Parameters
/// - `resource` (`SchemaResource`): The resource supplied to this operation.
///
/// # Returns
/// Type: `SchemaIssue`.
/// Returns `SchemaIssue`.
fn exceeded(resource: SchemaResource) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::LimitExceeded { resource })
}

/// Bounded, iterative traversal. Shared schema nodes do not discount repeated
/// value occurrences from depth/node/width limits.
///
/// # Parameters
/// - `value` (`&JsonValue`): Value read, transformed, encoded, or stored by this operation.
/// - `limits` (`&ParseLimits`): Explicit resource bounds enforced while processing this input.
///
/// # Returns
/// Type: `Result<(), SchemaIssue>`.
pub(crate) fn check_limits(value: &JsonValue, limits: &ParseLimits) -> Result<(), SchemaIssue> {
    let mut nodes = 0usize;
    let mut pending = vec![(value, 1usize)];
    while let Some((current, depth)) = pending.pop() {
        if depth > limits.depth.get() {
            return Err(exceeded(SchemaResource::Depth));
        }
        nodes = nodes
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Steps))?;
        if nodes > limits.nodes.get() {
            return Err(exceeded(SchemaResource::Steps));
        }
        match current {
            JsonValue::Array(values) => {
                if values.len() > limits.collection.get() {
                    return Err(exceeded(SchemaResource::Width));
                }
                let next = depth
                    .checked_add(1)
                    .ok_or_else(|| exceeded(SchemaResource::Depth))?;
                for item in values {
                    pending.push((item, next));
                }
            }
            JsonValue::Object(fields) => {
                if fields.len() > limits.collection.get() {
                    return Err(exceeded(SchemaResource::Width));
                }
                let next = depth
                    .checked_add(1)
                    .ok_or_else(|| exceeded(SchemaResource::Depth))?;
                for item in fields.values() {
                    pending.push((item, next));
                }
            }
            JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => {}
        }
    }
    let mut writer = BoundedWriter {
        count: 0,
        limit: limits.bytes.get(),
        exceeded: false,
    };
    let result = {
        let mut serializer = serde_json::Serializer::new(&mut writer);
        value.serialize(serde_stacker::Serializer::new(&mut serializer))
    };
    match result {
        Ok(()) => Ok(()),
        Err(_) if writer.exceeded => Err(exceeded(SchemaResource::Bytes)),
        Err(error) => Err(SchemaIssue::new(SchemaIssueKind::JsonSyntax {
            detail: error.to_string().into(),
        })),
    }
}

/// # Parameters
/// - `value` (`&JsonValue`): Value read, transformed, encoded, or stored by this operation.
/// - `limits` (`&ParseLimits`): Explicit resource bounds enforced while processing this input.
///
/// # Returns
/// Type: `Result<Vec<u8>, SchemaIssue>`.
pub(crate) fn encode_json_bytes(
    value: &JsonValue,
    limits: &ParseLimits,
) -> Result<Vec<u8>, SchemaIssue> {
    check_limits(value, limits)?;
    let mut bytes = Vec::new();
    let mut serializer = serde_json::Serializer::new(&mut bytes);
    value
        .serialize(serde_stacker::Serializer::new(&mut serializer))
        .map_err(|error| {
            SchemaIssue::new(SchemaIssueKind::JsonEncoding {
                detail: Arc::from(error.to_string()),
            })
        })?;
    Ok(bytes)
}

mod prepared;
pub use prepared::PreparedObjectAssembly;
pub(crate) use prepared::{AssemblyKind, PreparedArrayAssembly, PreparedDocumentAssembly, PreparedJsonParseReservation, PreparedWriteStageError};
pub use prepared::{
    PreparedJsonDocument, PreparedJsonKind, PreparedJsonParserCapacity, PreparedJsonParserDemand,
    PreparedJsonParserPool, PreparedJsonValue, SharedPreparedJsonValue,
};
