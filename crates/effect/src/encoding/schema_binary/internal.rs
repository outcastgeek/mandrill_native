use super::{BinaryError, BinaryLimits};
use crate::schema::{
    IssueSegment, JsonValue, PrimitiveShape, SchemaAst, SchemaIssue, SchemaIssueKind,
    SchemaResource, SchemaShape, StringLengthUnit, UnknownFields, ValueKind, value_kind,
};
use std::sync::Arc;

// Format-1 discriminants. They are wire definitions, not tunable policy.
const ABSENT: u8 = 0;
const PRESENT: u8 = 1;
const FALSE: u8 = 0;
const TRUE: u8 = 1;
const SIGNED: u8 = 0;
const UNSIGNED: u8 = 1;
const FLOAT: u8 = 2;
const ANY_NULL: u8 = 0;
const ANY_FALSE: u8 = 1;
const ANY_TRUE: u8 = 2;
const ANY_SIGNED: u8 = 3;
const ANY_UNSIGNED: u8 = 4;
const ANY_FLOAT: u8 = 5;
const ANY_STRING: u8 = 6;
const ANY_ARRAY: u8 = 7;
const ANY_OBJECT: u8 = 8;

fn issue(kind: SchemaIssueKind) -> BinaryError {
    BinaryError::Schema(SchemaIssue::new(kind))
}

fn mismatch(expected: ValueKind, actual: &JsonValue) -> BinaryError {
    issue(SchemaIssueKind::TypeMismatch {
        expected,
        actual: value_kind(actual),
    })
}

fn at(error: BinaryError, segment: IssueSegment) -> BinaryError {
    if let BinaryError::Schema(issue) = error {
        BinaryError::Schema(issue.at(segment))
    } else {
        error
    }
}

fn child_depth(depth: usize) -> Result<usize, BinaryError> {
    depth
        .checked_add(1)
        .ok_or(BinaryError::LimitExceeded(SchemaResource::Depth))
}

fn number_parts(value: &JsonValue, expected: ValueKind) -> Result<(u8, [u8; 8]), BinaryError> {
    let JsonValue::Number(number) = value else {
        return Err(mismatch(expected, value));
    };
    if let Some(signed) = number.as_i64() {
        Ok((SIGNED, signed.to_be_bytes()))
    } else if let Some(unsigned) = number.as_u64() {
        Ok((UNSIGNED, unsigned.to_be_bytes()))
    } else if let Some(float) = number.as_f64().filter(|value| value.is_finite()) {
        if serde_json::Number::from_f64(float).as_ref() != Some(number) {
            return Err(BinaryError::InvalidNumber);
        }
        Ok((FLOAT, float.to_be_bytes()))
    } else {
        Err(issue(SchemaIssueKind::NonFinite))
    }
}

#[derive(Clone, Copy)]
struct Budget {
    limits: BinaryLimits,
    nodes: usize,
    steps: usize,
}

impl Budget {
    fn value(&mut self, depth: usize) -> Result<(), BinaryError> {
        if depth > self.limits.depth.get() {
            return Err(BinaryError::LimitExceeded(SchemaResource::Depth));
        }
        self.nodes = self
            .nodes
            .checked_add(1)
            .ok_or(BinaryError::LengthOverflow)?;
        if self.nodes > self.limits.nodes.get() {
            return Err(BinaryError::LimitExceeded(SchemaResource::Steps));
        }
        Ok(())
    }

    fn step(&mut self) -> Result<(), BinaryError> {
        self.steps = self
            .steps
            .checked_add(1)
            .ok_or(BinaryError::LengthOverflow)?;
        if self.steps > self.limits.steps.get() {
            return Err(BinaryError::LimitExceeded(SchemaResource::Steps));
        }
        Ok(())
    }

    fn collection(&self, count: usize) -> Result<(), BinaryError> {
        if count > self.limits.collection.get() {
            Err(BinaryError::LimitExceeded(SchemaResource::Width))
        } else {
            Ok(())
        }
    }
}

pub(super) struct Output {
    pub(super) bytes: Vec<u8>,
    budget: Budget,
}

impl Output {
    pub(super) fn new(limits: BinaryLimits) -> Self {
        Self {
            bytes: Vec::new(),
            budget: Budget {
                limits,
                nodes: 0,
                steps: 0,
            },
        }
    }

    fn raw(&mut self, bytes: &[u8]) -> Result<(), BinaryError> {
        let next = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or(BinaryError::LengthOverflow)?;
        if next > self.budget.limits.bytes.get() {
            return Err(BinaryError::LimitExceeded(SchemaResource::Bytes));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    pub(super) fn byte(&mut self, byte: u8) -> Result<(), BinaryError> {
        self.raw(&[byte])
    }

    pub(super) fn u64(&mut self, value: u64) -> Result<(), BinaryError> {
        self.raw(&value.to_be_bytes())
    }

    fn length(&mut self, length: usize) -> Result<(), BinaryError> {
        self.u64(u64::try_from(length).map_err(|_| BinaryError::LengthOverflow)?)
    }

    pub(super) fn text(&mut self, text: &str) -> Result<(), BinaryError> {
        self.length(text.len())?;
        self.raw(text.as_bytes())
    }

    pub(super) fn value(
        &mut self,
        ast: &SchemaAst,
        value: &JsonValue,
        depth: usize,
    ) -> Result<(), BinaryError> {
        self.budget.value(depth)?;
        self.shape(ast, value, depth)
    }

    fn shape(
        &mut self,
        ast: &SchemaAst,
        value: &JsonValue,
        depth: usize,
    ) -> Result<(), BinaryError> {
        self.budget.step()?;
        match ast.shape() {
            SchemaShape::Opaque => Err(BinaryError::UnsupportedOpaque),
            SchemaShape::Primitive(primitive) => self.primitive(*primitive, value, depth),
            SchemaShape::Literal(literal) => {
                if value != literal.as_ref() {
                    return Err(issue(SchemaIssueKind::LiteralMismatch));
                }
                self.any(value, depth)
            }
            SchemaShape::Sequence { element, maximum } => {
                let JsonValue::Array(items) = value else {
                    return Err(mismatch(ValueKind::Sequence, value));
                };
                self.budget.collection(items.len())?;
                if maximum.is_some_and(|limit| items.len() > limit.get()) {
                    return Err(BinaryError::LimitExceeded(SchemaResource::Width));
                }
                self.length(items.len())?;
                let child = child_depth(depth)?;
                for (index, item) in items.iter().enumerate() {
                    self.value(element, item, child)
                        .map_err(|error| at(error, IssueSegment::Index(index)))?;
                }
                Ok(())
            }
            SchemaShape::Tuple(parts) => {
                let JsonValue::Array(items) = value else {
                    return Err(mismatch(ValueKind::Sequence, value));
                };
                self.budget.collection(parts.len())?;
                if items.len() != parts.len() {
                    return Err(issue(SchemaIssueKind::LengthMismatch {
                        expected: parts.len(),
                        actual: items.len(),
                    }));
                }
                let child = child_depth(depth)?;
                for (index, (part, item)) in parts.iter().zip(items).enumerate() {
                    self.value(part, item, child)
                        .map_err(|error| at(error, IssueSegment::Index(index)))?;
                }
                Ok(())
            }
            SchemaShape::Object {
                properties,
                unknown_fields,
            } => {
                let JsonValue::Object(object) = value else {
                    return Err(mismatch(ValueKind::Record, value));
                };
                self.budget.collection(object.len())?;
                self.budget.collection(properties.len())?;
                if *unknown_fields == UnknownFields::Reject {
                    for name in object.keys() {
                        if !properties.iter().any(|property| property.name() == name) {
                            return Err(issue(SchemaIssueKind::UnknownField(Arc::from(
                                name.as_str(),
                            ))));
                        }
                    }
                }
                let child = child_depth(depth)?;
                for property in properties {
                    match object.get(property.name()) {
                        Some(item) => {
                            self.byte(PRESENT)?;
                            self.value(property.schema(), item, child)
                                .map_err(|error| {
                                    at(error, IssueSegment::Field(Arc::from(property.name())))
                                })?;
                        }
                        None if property.required() && property.pure_default().is_none() => {
                            return Err(issue(SchemaIssueKind::MissingField(Arc::from(
                                property.name(),
                            ))));
                        }
                        None => self.byte(ABSENT)?,
                    }
                }
                Ok(())
            }
            SchemaShape::Union(variants) => {
                self.budget.collection(variants.len())?;
                let mut failures = Vec::new();
                for (index, variant) in variants.iter().enumerate() {
                    let start = self.bytes.len();
                    let candidate = self
                        .length(index)
                        .and_then(|()| self.shape(variant, value, depth));
                    match candidate {
                        Ok(()) => return Ok(()),
                        Err(error @ BinaryError::LimitExceeded(_)) => return Err(error),
                        Err(error) => {
                            self.bytes.truncate(start);
                            failures.push(error);
                        }
                    }
                }
                Err(BinaryError::NoUnionVariant(failures))
            }
            SchemaShape::Reference(reference) => {
                let target = reference
                    .resolve()
                    .ok_or(BinaryError::UnresolvedReference)?;
                self.shape(&target, value, depth)
            }
            SchemaShape::Checked { source, .. } | SchemaShape::EncodedView(source) => {
                self.shape(source, value, depth)
            }
            SchemaShape::IntegerRange {
                source,
                minimum,
                maximum,
            } => {
                let number = value
                    .as_i64()
                    .ok_or_else(|| mismatch(ValueKind::Integer, value))?;
                if number < *minimum || number > *maximum {
                    return Err(issue(SchemaIssueKind::OutOfRange));
                }
                self.shape(source, value, depth)
            }
            SchemaShape::UnsignedRange {
                source,
                minimum,
                maximum,
            } => {
                let number = value
                    .as_u64()
                    .ok_or_else(|| mismatch(ValueKind::Integer, value))?;
                if number < *minimum || number > *maximum {
                    return Err(issue(SchemaIssueKind::OutOfRange));
                }
                self.shape(source, value, depth)
            }
            SchemaShape::StringLength {
                source,
                minimum,
                maximum,
                unit,
            } => {
                check_string_length(value, *minimum, *maximum, *unit)?;
                self.shape(source, value, depth)
            }
            SchemaShape::StringPattern { source, pattern } => {
                let text = value
                    .as_str()
                    .ok_or_else(|| mismatch(ValueKind::String, value))?;
                if !pattern.matches(text) {
                    return Err(issue(SchemaIssueKind::PatternMismatch));
                }
                self.shape(source, value, depth)
            }
        }
    }

    fn primitive(
        &mut self,
        shape: PrimitiveShape,
        value: &JsonValue,
        depth: usize,
    ) -> Result<(), BinaryError> {
        match shape {
            PrimitiveShape::Any => self.any(value, depth),
            PrimitiveShape::Null if value.is_null() => Ok(()),
            PrimitiveShape::Null => Err(mismatch(ValueKind::Null, value)),
            PrimitiveShape::Boolean => {
                if let JsonValue::Bool(boolean) = value {
                    self.byte(u8::from(*boolean))
                } else {
                    Err(mismatch(ValueKind::Boolean, value))
                }
            }
            PrimitiveShape::String => {
                if let JsonValue::String(text) = value {
                    self.text(text)
                } else {
                    Err(mismatch(ValueKind::String, value))
                }
            }
            PrimitiveShape::Integer => {
                let (tag, bytes) = number_parts(value, ValueKind::Integer)?;
                if tag == FLOAT {
                    return Err(mismatch(ValueKind::Integer, value));
                }
                self.byte(tag)?;
                self.raw(&bytes)
            }
            PrimitiveShape::Number => {
                let (tag, bytes) = number_parts(value, ValueKind::Number)?;
                self.byte(tag)?;
                self.raw(&bytes)
            }
            PrimitiveShape::Binary => {
                let JsonValue::Array(items) = value else {
                    return Err(mismatch(ValueKind::Binary, value));
                };
                self.budget.collection(items.len())?;
                self.length(items.len())?;
                for (index, item) in items.iter().enumerate() {
                    let byte = item
                        .as_u64()
                        .and_then(|number| u8::try_from(number).ok())
                        .ok_or_else(|| {
                            at(
                                issue(SchemaIssueKind::OutOfRange),
                                IssueSegment::Index(index),
                            )
                        })?;
                    self.byte(byte)?;
                }
                Ok(())
            }
        }
    }

    fn any(&mut self, value: &JsonValue, depth: usize) -> Result<(), BinaryError> {
        self.budget.step()?;
        match value {
            JsonValue::Null => self.byte(ANY_NULL),
            JsonValue::Bool(false) => self.byte(ANY_FALSE),
            JsonValue::Bool(true) => self.byte(ANY_TRUE),
            JsonValue::Number(_) => {
                let (kind, bytes) = number_parts(value, ValueKind::Number)?;
                let tag = match kind {
                    SIGNED => ANY_SIGNED,
                    UNSIGNED => ANY_UNSIGNED,
                    FLOAT => ANY_FLOAT,
                    _ => return Err(BinaryError::InvalidTag(kind)),
                };
                self.byte(tag)?;
                self.raw(&bytes)
            }
            JsonValue::String(text) => {
                self.byte(ANY_STRING)?;
                self.text(text)
            }
            JsonValue::Array(items) => {
                self.budget.collection(items.len())?;
                self.byte(ANY_ARRAY)?;
                self.length(items.len())?;
                let child = child_depth(depth)?;
                for item in items {
                    self.budget.value(child)?;
                    self.any(item, child)?;
                }
                Ok(())
            }
            JsonValue::Object(object) => {
                self.budget.collection(object.len())?;
                self.byte(ANY_OBJECT)?;
                self.length(object.len())?;
                let child = child_depth(depth)?;
                for (name, item) in object {
                    self.text(name)?;
                    self.budget.value(child)?;
                    self.any(item, child)?;
                }
                Ok(())
            }
        }
    }
}

pub(super) struct Cursor<'a> {
    input: &'a [u8],
    pub(super) offset: usize,
    budget: Budget,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(input: &'a [u8], limits: BinaryLimits) -> Result<Self, BinaryError> {
        if input.len() > limits.bytes.get() {
            return Err(BinaryError::LimitExceeded(SchemaResource::Bytes));
        }
        Ok(Self {
            input,
            offset: 0,
            budget: Budget {
                limits,
                nodes: 0,
                steps: 0,
            },
        })
    }

    fn raw(&mut self, count: usize) -> Result<&'a [u8], BinaryError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(BinaryError::LengthOverflow)?;
        let bytes = self
            .input
            .get(self.offset..end)
            .ok_or(BinaryError::Truncated {
                offset: self.offset,
                needed: count,
            })?;
        self.offset = end;
        Ok(bytes)
    }

    pub(super) fn byte(&mut self) -> Result<u8, BinaryError> {
        let offset = self.offset;
        self.raw(1)?
            .first()
            .copied()
            .ok_or(BinaryError::Truncated { offset, needed: 1 })
    }

    pub(super) fn u64(&mut self) -> Result<u64, BinaryError> {
        let mut bytes = [0; size_of::<u64>()];
        let length = bytes.len();
        bytes.copy_from_slice(self.raw(length)?);
        Ok(u64::from_be_bytes(bytes))
    }

    fn length(&mut self) -> Result<usize, BinaryError> {
        usize::try_from(self.u64()?).map_err(|_| BinaryError::LengthOverflow)
    }

    pub(super) fn text(&mut self) -> Result<&'a str, BinaryError> {
        let count = self.length()?;
        std::str::from_utf8(self.raw(count)?).map_err(|_| BinaryError::InvalidUtf8)
    }

    pub(super) fn value(
        &mut self,
        ast: &SchemaAst,
        depth: usize,
    ) -> Result<JsonValue, BinaryError> {
        self.budget.value(depth)?;
        self.shape(ast, depth)
    }

    fn shape(&mut self, ast: &SchemaAst, depth: usize) -> Result<JsonValue, BinaryError> {
        self.budget.step()?;
        match ast.shape() {
            SchemaShape::Opaque => Err(BinaryError::UnsupportedOpaque),
            SchemaShape::Primitive(primitive) => self.primitive(*primitive, depth),
            SchemaShape::Literal(literal) => {
                let value = self.any(depth)?;
                if value == *literal.as_ref() {
                    Ok(value)
                } else {
                    Err(issue(SchemaIssueKind::LiteralMismatch))
                }
            }
            SchemaShape::Sequence { element, maximum } => {
                let count = self.length()?;
                self.budget.collection(count)?;
                if maximum.is_some_and(|limit| count > limit.get()) {
                    return Err(BinaryError::LimitExceeded(SchemaResource::Width));
                }
                let mut items = Vec::new();
                let child = child_depth(depth)?;
                for index in 0..count {
                    items.push(
                        self.value(element, child)
                            .map_err(|error| at(error, IssueSegment::Index(index)))?,
                    );
                }
                Ok(JsonValue::Array(items))
            }
            SchemaShape::Tuple(parts) => {
                self.budget.collection(parts.len())?;
                let mut items = Vec::new();
                let child = child_depth(depth)?;
                for (index, part) in parts.iter().enumerate() {
                    items.push(
                        self.value(part, child)
                            .map_err(|error| at(error, IssueSegment::Index(index)))?,
                    );
                }
                Ok(JsonValue::Array(items))
            }
            SchemaShape::Object { properties, .. } => {
                self.budget.collection(properties.len())?;
                let mut object = serde_json::Map::new();
                let child = child_depth(depth)?;
                for property in properties {
                    match self.byte()? {
                        ABSENT if property.required() && property.pure_default().is_none() => {
                            return Err(issue(SchemaIssueKind::MissingField(Arc::from(
                                property.name(),
                            ))));
                        }
                        ABSENT => {}
                        PRESENT => {
                            let item = self.value(property.schema(), child).map_err(|error| {
                                at(error, IssueSegment::Field(Arc::from(property.name())))
                            })?;
                            object.insert(property.name().to_owned(), item);
                        }
                        tag => return Err(BinaryError::InvalidTag(tag)),
                    }
                }
                Ok(JsonValue::Object(object))
            }
            SchemaShape::Union(variants) => {
                self.budget.collection(variants.len())?;
                let index = self.u64()?;
                let index_usize =
                    usize::try_from(index).map_err(|_| BinaryError::InvalidUnionIndex(index))?;
                let variant = variants
                    .get(index_usize)
                    .ok_or(BinaryError::InvalidUnionIndex(index))?;
                self.shape(variant, depth)
            }
            SchemaShape::Reference(reference) => {
                let target = reference
                    .resolve()
                    .ok_or(BinaryError::UnresolvedReference)?;
                self.shape(&target, depth)
            }
            SchemaShape::Checked { source, .. } | SchemaShape::EncodedView(source) => {
                self.shape(source, depth)
            }
            SchemaShape::IntegerRange {
                source,
                minimum,
                maximum,
            } => {
                let value = self.shape(source, depth)?;
                let number = value
                    .as_i64()
                    .ok_or_else(|| mismatch(ValueKind::Integer, &value))?;
                if number < *minimum || number > *maximum {
                    return Err(issue(SchemaIssueKind::OutOfRange));
                }
                Ok(value)
            }
            SchemaShape::UnsignedRange {
                source,
                minimum,
                maximum,
            } => {
                let value = self.shape(source, depth)?;
                let number = value
                    .as_u64()
                    .ok_or_else(|| mismatch(ValueKind::Integer, &value))?;
                if number < *minimum || number > *maximum {
                    return Err(issue(SchemaIssueKind::OutOfRange));
                }
                Ok(value)
            }
            SchemaShape::StringLength {
                source,
                minimum,
                maximum,
                unit,
            } => {
                let value = self.shape(source, depth)?;
                check_string_length(&value, *minimum, *maximum, *unit)?;
                Ok(value)
            }
            SchemaShape::StringPattern { source, pattern } => {
                let value = self.shape(source, depth)?;
                let text = value
                    .as_str()
                    .ok_or_else(|| mismatch(ValueKind::String, &value))?;
                if !pattern.matches(text) {
                    return Err(issue(SchemaIssueKind::PatternMismatch));
                }
                Ok(value)
            }
        }
    }

    fn primitive(&mut self, shape: PrimitiveShape, depth: usize) -> Result<JsonValue, BinaryError> {
        match shape {
            PrimitiveShape::Any => self.any(depth),
            PrimitiveShape::Null => Ok(JsonValue::Null),
            PrimitiveShape::Boolean => match self.byte()? {
                FALSE => Ok(JsonValue::Bool(false)),
                TRUE => Ok(JsonValue::Bool(true)),
                tag => Err(BinaryError::InvalidTag(tag)),
            },
            PrimitiveShape::String => Ok(JsonValue::String(self.text()?.to_owned())),
            PrimitiveShape::Integer => self.number(false),
            PrimitiveShape::Number => self.number(true),
            PrimitiveShape::Binary => {
                let count = self.length()?;
                self.budget.collection(count)?;
                let bytes = self.raw(count)?;
                Ok(JsonValue::Array(
                    bytes
                        .iter()
                        .map(|byte| JsonValue::Number(serde_json::Number::from(u64::from(*byte))))
                        .collect(),
                ))
            }
        }
    }

    fn number(&mut self, allow_float: bool) -> Result<JsonValue, BinaryError> {
        let tag = self.byte()?;
        self.number_payload(tag, allow_float)
    }

    fn number_payload(&mut self, tag: u8, allow_float: bool) -> Result<JsonValue, BinaryError> {
        if tag > FLOAT || (tag == FLOAT && !allow_float) {
            return Err(BinaryError::InvalidTag(tag));
        }
        let mut bytes = [0; size_of::<u64>()];
        let length = bytes.len();
        bytes.copy_from_slice(self.raw(length)?);
        let number = match tag {
            SIGNED => serde_json::Number::from(i64::from_be_bytes(bytes)),
            UNSIGNED => serde_json::Number::from(u64::from_be_bytes(bytes)),
            FLOAT if allow_float => serde_json::Number::from_f64(f64::from_be_bytes(bytes))
                .ok_or(BinaryError::InvalidNumber)?,
            _ => return Err(BinaryError::InvalidTag(tag)),
        };
        Ok(JsonValue::Number(number))
    }

    fn any(&mut self, depth: usize) -> Result<JsonValue, BinaryError> {
        self.budget.step()?;
        match self.byte()? {
            ANY_NULL => Ok(JsonValue::Null),
            ANY_FALSE => Ok(JsonValue::Bool(false)),
            ANY_TRUE => Ok(JsonValue::Bool(true)),
            ANY_SIGNED => self.number_payload(SIGNED, true),
            ANY_UNSIGNED => self.number_payload(UNSIGNED, true),
            ANY_FLOAT => self.number_payload(FLOAT, true),
            ANY_STRING => Ok(JsonValue::String(self.text()?.to_owned())),
            ANY_ARRAY => {
                let count = self.length()?;
                self.budget.collection(count)?;
                let mut items = Vec::new();
                let child = child_depth(depth)?;
                for _ in 0..count {
                    self.budget.value(child)?;
                    items.push(self.any(child)?);
                }
                Ok(JsonValue::Array(items))
            }
            ANY_OBJECT => {
                let count = self.length()?;
                self.budget.collection(count)?;
                let mut object = serde_json::Map::new();
                let child = child_depth(depth)?;
                for _ in 0..count {
                    let name = self.text()?.to_owned();
                    self.budget.value(child)?;
                    let value = self.any(child)?;
                    if object.contains_key(&name) {
                        return Err(BinaryError::DuplicateKey(name));
                    }
                    object.insert(name, value);
                }
                Ok(JsonValue::Object(object))
            }
            tag => Err(BinaryError::InvalidTag(tag)),
        }
    }
}

fn check_string_length(
    value: &JsonValue,
    minimum: usize,
    maximum: usize,
    unit: StringLengthUnit,
) -> Result<(), BinaryError> {
    let JsonValue::String(text) = value else {
        return Err(mismatch(ValueKind::String, value));
    };
    let length = match unit {
        StringLengthUnit::Utf8Bytes => text.len(),
        StringLengthUnit::UnicodeScalars => text.chars().count(),
        StringLengthUnit::Graphemes => return Err(BinaryError::UnsupportedGraphemeLength),
    };
    if length < minimum || length > maximum {
        Err(issue(SchemaIssueKind::OutOfRange))
    } else {
        Ok(())
    }
}
