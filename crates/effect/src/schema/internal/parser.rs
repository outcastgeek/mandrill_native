//! Bounded exact JSON parsing and structural checks.
use super::super::{
    DuplicateFields, JsonValue, ParseLimits, ParsePolicy, SchemaErrorMode, SchemaIssue,
    SchemaIssueKind, SchemaResource, UnknownFields,
};
use serde::Serialize;
use std::io::{self, Write};
use std::sync::Arc;

enum Container {
    Array {
        values: Vec<JsonValue>,
        need_value: bool,
        may_end: bool,
    },
    Object {
        fields: serde_json::Map<String, JsonValue>,
        key: Option<String>,
        need_key: bool,
        may_end: bool,
        count: usize,
    },
}
struct Tokens<'a> {
    text: &'a str,
    offset: usize,
}
impl Tokens<'_> {
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
        SchemaIssue::new(SchemaIssueKind::JsonSyntax {
            detail: Arc::from(format!("invalid JSON at byte {}", self.offset)),
        })
    }
    fn string(&mut self) -> Result<String, SchemaIssue> {
        let start = self.offset;
        self.offset += 1;
        let mut escaped = false;
        while let Some(byte) = self.text.as_bytes().get(self.offset).copied() {
            self.offset += 1;
            if !escaped && byte == b'"' {
                return serde_json::from_str(
                    self.text
                        .get(start..self.offset)
                        .ok_or_else(|| self.syntax())?,
                )
                .map_err(|_| self.syntax());
            }
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            }
        }
        Err(self.syntax())
    }
    fn scalar(&mut self) -> Result<JsonValue, SchemaIssue> {
        if self.byte() == Some(b'"') {
            return self.string().map(JsonValue::String);
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
        match token {
            "null" => Ok(JsonValue::Null),
            "true" => Ok(JsonValue::Bool(true)),
            "false" => Ok(JsonValue::Bool(false)),
            _ => token
                .parse::<serde_json::Number>()
                .map(JsonValue::Number)
                .map_err(|_| self.syntax()),
        }
    }
}
fn locate(issue: SchemaIssue, stack: &[Container]) -> SchemaIssue {
    stack
        .iter()
        .rev()
        .fold(issue, |issue, container| match container {
            Container::Array { values, .. } => {
                issue.at(super::super::IssueSegment::Index(values.len()))
            }
            Container::Object {
                key: Some(name), ..
            } => issue.at(super::super::IssueSegment::Field(name.as_str().into())),
            Container::Object { key: None, .. } => issue,
        })
}
pub(crate) fn parse_json(text: &str, policy: ParsePolicy) -> Result<JsonValue, SchemaIssue> {
    if text.len() > policy.limits.bytes.get() {
        return Err(exceeded(SchemaResource::Bytes));
    }
    let mut tokens = Tokens { text, offset: 0 };
    let mut stack: Vec<Container> = Vec::new();
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
                    if values.len() == policy.limits.collection.get() {
                        return Err(exceeded(SchemaResource::Width));
                    }
                    values.push(value);
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
                    let duplicate = fields.contains_key(&name);
                    if duplicate && policy.duplicate_fields == DuplicateFields::Reject {
                        return Err(SchemaIssue::new(SchemaIssueKind::DuplicateField(
                            name.into(),
                        )));
                    }
                    if !duplicate || policy.duplicate_fields == DuplicateFields::KeepLast {
                        fields.insert(name, value);
                    }
                    *need_key = false;
                    *may_end = true;
                }
                None => {
                    if tokens.byte().is_some() {
                        return Err(tokens.syntax());
                    }
                    check_limits(&value, &policy.limits)?;
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
                        completed = Some(JsonValue::Array(values));
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
                    completed = Some(JsonValue::Array(values));
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
                        completed = Some(JsonValue::Object(fields));
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
                    completed = Some(JsonValue::Object(fields));
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
                let name = tokens.string()?;
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
            return Err(locate(exceeded(SchemaResource::Depth), &stack));
        }
        nodes = nodes
            .checked_add(1)
            .ok_or_else(|| exceeded(SchemaResource::Steps))?;
        if nodes > policy.limits.nodes.get() {
            return Err(locate(exceeded(SchemaResource::Steps), &stack));
        }
        match byte {
            b'[' => {
                tokens.offset += 1;
                stack.push(Container::Array {
                    values: Vec::new(),
                    need_value: true,
                    may_end: true,
                });
            }
            b'{' => {
                tokens.offset += 1;
                stack.push(Container::Object {
                    fields: serde_json::Map::new(),
                    key: None,
                    need_key: true,
                    may_end: true,
                    count: 0,
                });
            }
            _ => completed = Some(tokens.scalar()?),
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
