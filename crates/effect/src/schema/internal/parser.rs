use super::super::{
    DuplicateFields, JsonValue, ParseLimits, ParsePolicy, SchemaErrorMode, SchemaIssue,
    SchemaIssueKind, SchemaResource, UnknownFields,
};
use serde::Serialize;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::cell::Cell;
use std::cell::RefCell;
use std::fmt;
use std::io::{self, Write};
use std::sync::Arc;

#[derive(Clone, Copy)]
struct JsonSeed<'a> {
    policy: ParsePolicy,
    depth: usize,
    nodes: &'a Cell<usize>,
    failure: &'a RefCell<Option<SchemaIssue>>,
}

impl<'a> JsonSeed<'a> {
    fn reject<Error: de::Error>(&self, issue: SchemaIssue) -> Error {
        let error = Error::custom(issue.to_string());
        *self.failure.borrow_mut() = Some(issue);
        error
    }

    fn child<Error: de::Error>(&self) -> Result<Self, Error> {
        let depth = self.depth.checked_add(1).ok_or_else(|| {
            self.reject(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                resource: SchemaResource::Depth,
            }))
        })?;
        Ok(Self { depth, ..*self })
    }
}

impl<'de> DeserializeSeed<'de> for JsonSeed<'_> {
    type Value = JsonValue;

    fn deserialize<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        if self.depth > self.policy.limits.depth.get() {
            return Err(
                self.reject(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: SchemaResource::Depth,
                })),
            );
        }
        let nodes = self.nodes.get().checked_add(1).ok_or_else(|| {
            self.reject(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                resource: SchemaResource::Steps,
            }))
        })?;
        if nodes > self.policy.limits.nodes.get() {
            return Err(
                self.reject(SchemaIssue::new(SchemaIssueKind::LimitExceeded {
                    resource: SchemaResource::Steps,
                })),
            );
        }
        self.nodes.set(nodes);
        deserializer.deserialize_any(self)
    }
}

struct OverflowSeed<'a> {
    failure: &'a RefCell<Option<SchemaIssue>>,
}

impl<'de> DeserializeSeed<'de> for OverflowSeed<'_> {
    type Value = ();

    fn deserialize<D: de::Deserializer<'de>>(self, _: D) -> Result<Self::Value, D::Error> {
        let issue = exceeded(SchemaResource::Width);
        let error = <D::Error as de::Error>::custom(issue.to_string());
        *self.failure.borrow_mut() = Some(issue);
        Err(error)
    }
}

impl<'de> Visitor<'de> for JsonSeed<'_> {
    type Value = JsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(JsonValue::Null)
    }
    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(JsonValue::Null)
    }
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(JsonValue::Bool(value))
    }
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(JsonValue::Number(value.into()))
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(JsonValue::Number(value.into()))
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        serde_json::Number::from_f64(value)
            .map(JsonValue::Number)
            .ok_or_else(|| self.reject(SchemaIssue::new(SchemaIssueKind::NonFinite)))
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(JsonValue::String(value.to_owned()))
    }
    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(JsonValue::String(value))
    }

    fn visit_seq<S: SeqAccess<'de>>(self, mut sequence: S) -> Result<Self::Value, S::Error> {
        let child = self.child()?;
        let mut values = Vec::new();
        loop {
            if values.len() == self.policy.limits.collection.get() {
                match sequence.next_element_seed(OverflowSeed {
                    failure: self.failure,
                }) {
                    Ok(None) => break,
                    Ok(Some(())) => return Err(self.reject(exceeded(SchemaResource::Width))),
                    Err(error) => return Err(error),
                }
            }
            match sequence.next_element_seed(JsonSeed { ..child })? {
                Some(value) => values.push(value),
                None => break,
            }
        }
        Ok(JsonValue::Array(values))
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
        let child = self.child()?;
        let mut object = serde_json::Map::new();
        let mut seen_fields = 0usize;
        while let Some(key) = map.next_key::<String>()? {
            seen_fields = seen_fields
                .checked_add(1)
                .ok_or_else(|| self.reject(exceeded(SchemaResource::Width)))?;
            if seen_fields > self.policy.limits.collection.get() {
                return Err(self.reject(exceeded(SchemaResource::Width)));
            }
            let duplicate = object.contains_key(&key);
            if duplicate && self.policy.duplicate_fields == DuplicateFields::Reject {
                return Err(
                    self.reject(SchemaIssue::new(SchemaIssueKind::DuplicateField(
                        Arc::from(key),
                    ))),
                );
            }
            let value = map.next_value_seed(JsonSeed { ..child })?;
            if !duplicate || self.policy.duplicate_fields == DuplicateFields::KeepLast {
                object.insert(key, value);
            }
        }
        Ok(JsonValue::Object(object))
    }
}

pub(crate) fn parse_json(text: &str, policy: ParsePolicy) -> Result<JsonValue, SchemaIssue> {
    if text.len() > policy.limits.bytes.get() {
        return Err(exceeded(SchemaResource::Bytes));
    }
    let nodes = Cell::new(0usize);
    let failure = RefCell::new(None);
    let seed = JsonSeed {
        policy,
        depth: 1,
        nodes: &nodes,
        failure: &failure,
    };
    let mut deserializer = serde_json::Deserializer::from_str(text);
    deserializer.disable_recursion_limit();
    let parsed = seed
        .deserialize(serde_stacker::Deserializer::new(&mut deserializer))
        .and_then(|value| deserializer.end().map(|_| value));
    match parsed {
        Ok(value) => {
            check_limits(&value, &policy.limits)?;
            Ok(value)
        }
        Err(error) => Err(failure.into_inner().unwrap_or_else(|| {
            SchemaIssue::new(SchemaIssueKind::JsonSyntax {
                detail: Arc::from(error.to_string()),
            })
        })),
    }
}

/// Fixed versioned wire records reject duplicate fields before constructing
/// a value. Their own decoders validate the exact allowed field set.
pub(crate) const fn strict_wire_policy(limits: ParseLimits) -> ParsePolicy {
    ParsePolicy {
        limits,
        errors: SchemaErrorMode::First,
        unknown_fields: UnknownFields::Reject,
        duplicate_fields: DuplicateFields::Reject,
    }
}

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

struct BoundedWriter {
    count: usize,
    limit: usize,
    exceeded: bool,
}

impl Write for BoundedWriter {
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

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn exceeded(resource: SchemaResource) -> SchemaIssue {
    SchemaIssue::new(SchemaIssueKind::LimitExceeded { resource })
}

/// Bounded, iterative traversal. Shared schema nodes do not discount repeated
/// value occurrences from depth/node/width limits.
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
