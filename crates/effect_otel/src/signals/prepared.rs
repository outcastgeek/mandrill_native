//! Physical typed attribute records held through their native handoff.

use super::NativeError;
use effect::memory::{Buffer, BufferLease, Bytes, MemoryError, PreparedMemory, RegionId, SlotPool};
use effect::{Annotations, Defect, LibraryDefect};
use effect_tracing::{PreparedSpanAnnotations, PreparedTracingCapacity};
use opentelemetry::{Array, KeyValue, StringValue, Value};
use std::alloc::Layout;
use std::num::NonZeroUsize;
use std::sync::Arc;

const WORD: usize = std::mem::size_of::<usize>();
const BOOL: u8 = 0;
const I64: u8 = 1;
const F64: u8 = 2;
const STRING: u8 = 3;
const BOOL_ARRAY: u8 = 4;
const I64_ARRAY: u8 = 5;
const F64_ARRAY: u8 = 6;
const STRING_ARRAY: u8 = 7;

type AnnotationRedactor =
    dyn Fn(&Annotations, &mut PreparedSpanAnnotations<'_>) -> Result<(), Defect> + Send + Sync;
type AttributeRedactor =
    dyn Fn(&[KeyValue], &mut PreparedAttributeWriter<'_>) -> Result<(), Defect> + Send + Sync;

/// Explicit prepared span and transient native-signal staging limits.
#[derive(Clone, Copy, Debug)]
pub struct PreparedOtelSignalCapacity {
    pub tracing: PreparedTracingCapacity,
    /// Concurrent redaction and native handoffs for span options, events, and attributes.
    pub records: NonZeroUsize,
    /// Maximum packed record bytes, including names, typed values and lengths.
    pub record_bytes: Bytes,
    /// Maximum name bytes per event.
    pub event_name_bytes: Bytes,
    /// Maximum total attributes in a record including every linked group.
    pub attributes: usize,
    /// Maximum number of links admitted by a span start.
    pub links: usize,
}

impl PreparedOtelSignalCapacity {
    /// Regional span and record payload, excluding allocator overhead and native SDK copies.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        let layout =
            Layout::array::<u8>(self.record_bytes.get()).map_err(|_| MemoryError::InvalidLayout)?;
        self.tracing
            .storage_bytes()?
            .checked_add(SlotPool::<Buffer<'static>>::storage_bytes(
                self.records.get(),
            )?)?
            .checked_add(Bytes::new(layout.size()).checked_mul(self.records.get())?)
    }
}

/// Caller redaction writes approved output into each physically admitted record.
#[derive(Clone)]
pub struct PreparedOtelRedaction {
    pub(super) annotations: Arc<AnnotationRedactor>,
    attributes: Arc<AttributeRedactor>,
}

impl PreparedOtelRedaction {
    #[must_use]
    pub fn new(
        annotations: impl Fn(&Annotations, &mut PreparedSpanAnnotations<'_>) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
        attributes: impl Fn(&[KeyValue], &mut PreparedAttributeWriter<'_>) -> Result<(), Defect>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            annotations: Arc::new(annotations),
            attributes: Arc::new(attributes),
        }
    }
}

pub(super) struct PreparedSignals {
    capacity: PreparedOtelSignalCapacity,
    slots: SlotPool<'static, Buffer<'static>>,
    pub(super) redact: PreparedOtelRedaction,
}

impl PreparedSignals {
    pub(super) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedOtelSignalCapacity,
        redact: PreparedOtelRedaction,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.storage_bytes()?;
        Ok(Self {
            slots: memory.buffers(region, capacity.records.get(), capacity.record_bytes)?,
            capacity,
            redact,
        })
    }

    pub(super) fn stage<'a>(
        &self,
        name: &str,
        attributes: &'a [KeyValue],
        links: &'a [opentelemetry::trace::Link],
    ) -> Result<SignalRecord, NativeError> {
        let mut buffer = self.slots.try_checkout().map_err(fail)?;
        if name.len() > self.capacity.event_name_bytes.get() || links.len() > self.capacity.links {
            return Err(fail(MemoryError::CollectionReservationFailed));
        }
        let groups = || {
            std::iter::once(attributes).chain(links.iter().map(|link| link.attributes.as_slice()))
        };
        let mut count = 0usize;
        let mut bytes = length_prefixed(name).map_err(fail)?;
        for group in groups() {
            count = count
                .checked_add(group.len())
                .ok_or_else(|| fail(MemoryError::ArithmeticOverflow))?;
            if count > self.capacity.attributes {
                return Err(fail(MemoryError::CollectionReservationFailed));
            }
            bytes = bytes.checked_add(Bytes::new(WORD)).map_err(fail)?;
            for value in group {
                bytes = bytes
                    .checked_add(
                        attribute_bytes(value.key.as_str(), Borrowed::Native(&value.value))
                            .map_err(fail)?,
                    )
                    .map_err(fail)?;
            }
            if bytes > self.capacity.record_bytes {
                return Err(fail(MemoryError::ValueTooLarge {
                    requested: bytes,
                    capacity: self.capacity.record_bytes,
                }));
            }
        }

        buffer.clear();
        append_text(&mut buffer, name).map_err(fail)?;
        let mut remaining = self.capacity.attributes;
        for group in groups() {
            let start = buffer.len();
            buffer
                .extend_from_slice(&0usize.to_ne_bytes())
                .map_err(fail)?;
            let mut writer = PreparedAttributeWriter {
                buffer: &mut buffer,
                remaining,
                failure: None,
            };
            (self.redact.attributes)(group, &mut writer).map_err(NativeError::Native)?;
            if let Some(error) = writer.failure {
                return Err(fail(error));
            }
            remaining = writer.remaining;
            let size = buffer.len() - start - WORD;
            buffer
                .as_mut_slice()
                .get_mut(start..start + WORD)
                .ok_or_else(invalid)?
                .copy_from_slice(&size.to_ne_bytes());
        }
        Ok(SignalRecord { buffer })
    }
}

/// A checked destination for the selected typed attribute redactor.
pub struct PreparedAttributeWriter<'a> {
    buffer: &'a mut Buffer<'static>,
    remaining: usize,
    failure: Option<MemoryError>,
}

impl PreparedAttributeWriter<'_> {
    /// Copy approved native typed data into the admitted record without cloning
    /// the SDK's owned strings or vectors.
    pub fn push(&mut self, value: &KeyValue) -> Result<(), MemoryError> {
        self.push_value(value.key.as_str(), Borrowed::Native(&value.value))
    }

    /// Write a borrowed redacted string, including replacements produced from
    /// static policy, without making a native KeyValue first.
    pub fn push_string(&mut self, key: &str, value: &str) -> Result<(), MemoryError> {
        self.push_value(key, Borrowed::String(value))
    }

    fn push_value(&mut self, key: &str, value: Borrowed<'_>) -> Result<(), MemoryError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        let result = self.push_checked(key, value);
        if let Err(error) = result {
            self.failure = Some(error);
        }
        result
    }

    fn push_checked(&mut self, key: &str, value: Borrowed<'_>) -> Result<(), MemoryError> {
        if self.remaining == 0 {
            return Err(MemoryError::CollectionReservationFailed);
        }
        let requested = Bytes::new(self.buffer.len()).checked_add(attribute_bytes(key, value)?)?;
        let capacity = self.buffer.capacity();
        if requested > capacity {
            return Err(MemoryError::ValueTooLarge {
                requested,
                capacity,
            });
        }
        append_text(self.buffer, key)?;
        encode(self.buffer, value)?;
        self.remaining -= 1;
        Ok(())
    }
}

pub(super) struct SignalRecord {
    buffer: BufferLease<'static>,
}

impl SignalRecord {
    pub(super) fn parts(&self) -> Result<(&str, Groups<'_>), NativeError> {
        let mut bytes = self.buffer.as_slice();
        let name = read_text(&mut bytes)?;
        Ok((name, Groups(bytes)))
    }
}

pub(super) struct Groups<'a>(&'a [u8]);

impl<'a> Groups<'a> {
    pub(super) fn next(&mut self) -> Result<Attributes<'a>, NativeError> {
        let length = usize::from_ne_bytes(take(&mut self.0)?);
        let (group, remaining) = self.0.split_at_checked(length).ok_or_else(invalid)?;
        self.0 = remaining;
        Ok(Attributes(group))
    }
}

pub(super) struct Attributes<'a>(&'a [u8]);

impl Attributes<'_> {
    pub(super) fn for_each(mut self, mut emit: impl FnMut(KeyValue)) -> Result<(), NativeError> {
        while !self.0.is_empty() {
            let key = read_text(&mut self.0)?;
            let value = decode(&mut self.0)?;
            emit(KeyValue::new(key.to_owned(), value));
        }
        Ok(())
    }

    /// SDK event and link calls require an owned Vec; materialize it only at
    /// handoff from the already admitted bounded record.
    pub(super) fn native(self) -> Result<Vec<KeyValue>, NativeError> {
        let mut output = Vec::new();
        self.for_each(|value| output.push(value))?;
        Ok(output)
    }
}

#[derive(Clone, Copy)]
enum Borrowed<'a> {
    Native(&'a Value),
    String(&'a str),
}

fn length_prefixed(text: &str) -> Result<Bytes, MemoryError> {
    Bytes::new(WORD).checked_add(Bytes::new(text.len()))
}

fn attribute_bytes(key: &str, value: Borrowed<'_>) -> Result<Bytes, MemoryError> {
    let size = match value {
        Borrowed::String(text) => length_prefixed(text)?,
        Borrowed::Native(value) => match value {
            Value::Bool(_) => Bytes::new(1),
            Value::I64(_) | Value::F64(_) => Bytes::new(8),
            Value::String(text) => length_prefixed(text.as_str())?,
            Value::Array(array) => match array {
                Array::Bool(values) => Bytes::new(WORD).checked_add(Bytes::new(values.len()))?,
                Array::I64(values) => array_bytes(values.len())?,
                Array::F64(values) => array_bytes(values.len())?,
                Array::String(values) => {
                    values.iter().try_fold(Bytes::new(WORD), |bytes, text| {
                        bytes.checked_add(length_prefixed(text.as_str())?)
                    })?
                }
                _ => return Err(MemoryError::UnsupportedRunContract),
            },
            _ => return Err(MemoryError::UnsupportedRunContract),
        },
    };
    length_prefixed(key)?
        .checked_add(Bytes::new(1))?
        .checked_add(size)
}

fn array_bytes(count: usize) -> Result<Bytes, MemoryError> {
    Bytes::new(WORD).checked_add(Bytes::new(8).checked_mul(count)?)
}

fn append_text(buffer: &mut Buffer<'_>, text: &str) -> Result<(), MemoryError> {
    buffer.extend_from_slice(&text.len().to_ne_bytes())?;
    buffer.extend_from_slice(text.as_bytes())
}

fn encode(buffer: &mut Buffer<'_>, value: Borrowed<'_>) -> Result<(), MemoryError> {
    let tag = match value {
        Borrowed::String(_) | Borrowed::Native(Value::String(_)) => STRING,
        Borrowed::Native(Value::Bool(_)) => BOOL,
        Borrowed::Native(Value::I64(_)) => I64,
        Borrowed::Native(Value::F64(_)) => F64,
        Borrowed::Native(Value::Array(Array::Bool(_))) => BOOL_ARRAY,
        Borrowed::Native(Value::Array(Array::I64(_))) => I64_ARRAY,
        Borrowed::Native(Value::Array(Array::F64(_))) => F64_ARRAY,
        Borrowed::Native(Value::Array(Array::String(_))) => STRING_ARRAY,
        Borrowed::Native(_) => return Err(MemoryError::UnsupportedRunContract),
    };
    buffer.extend_from_slice(&[tag])?;
    match value {
        Borrowed::String(text) => append_text(buffer, text),
        Borrowed::Native(value) => match value {
            Value::Bool(value) => buffer.extend_from_slice(&[u8::from(*value)]),
            Value::I64(value) => buffer.extend_from_slice(&value.to_ne_bytes()),
            Value::F64(value) => buffer.extend_from_slice(&value.to_ne_bytes()),
            Value::String(value) => append_text(buffer, value.as_str()),
            Value::Array(array) => match array {
                Array::Bool(values) => write_array(buffer, values, |buffer, value| {
                    buffer.extend_from_slice(&[u8::from(*value)])
                }),
                Array::I64(values) => write_array(buffer, values, |buffer, value| {
                    buffer.extend_from_slice(&value.to_ne_bytes())
                }),
                Array::F64(values) => write_array(buffer, values, |buffer, value| {
                    buffer.extend_from_slice(&value.to_ne_bytes())
                }),
                Array::String(values) => write_array(buffer, values, |buffer, value| {
                    append_text(buffer, value.as_str())
                }),
                _ => Err(MemoryError::UnsupportedRunContract),
            },
            _ => Err(MemoryError::UnsupportedRunContract),
        },
    }
}

fn write_array<T>(
    buffer: &mut Buffer<'_>,
    values: &[T],
    write: impl Fn(&mut Buffer<'_>, &T) -> Result<(), MemoryError>,
) -> Result<(), MemoryError> {
    buffer.extend_from_slice(&values.len().to_ne_bytes())?;
    for value in values {
        write(buffer, value)?;
    }
    Ok(())
}

fn take<const N: usize>(bytes: &mut &[u8]) -> Result<[u8; N], NativeError> {
    let (content, rest) = bytes.split_first_chunk::<N>().ok_or_else(invalid)?;
    *bytes = rest;
    Ok(*content)
}

fn read_text<'a>(bytes: &mut &'a [u8]) -> Result<&'a str, NativeError> {
    let size = usize::from_ne_bytes(take(bytes)?);
    let (text, rest) = bytes.split_at_checked(size).ok_or_else(invalid)?;
    *bytes = rest;
    str::from_utf8(text).map_err(|_| invalid())
}

fn decode(bytes: &mut &[u8]) -> Result<Value, NativeError> {
    let [tag] = take(bytes)?;
    match tag {
        BOOL => Ok(Value::Bool(take::<1>(bytes)? == [1])),
        I64 => Ok(Value::I64(i64::from_ne_bytes(take(bytes)?))),
        F64 => Ok(Value::F64(f64::from_ne_bytes(take(bytes)?))),
        STRING => Ok(Value::String(StringValue::from(
            read_text(bytes)?.to_owned(),
        ))),
        BOOL_ARRAY => read_array(bytes, |bytes| Ok(take::<1>(bytes)? == [1]))
            .map(Array::Bool)
            .map(Value::Array),
        I64_ARRAY => read_array(bytes, |bytes| Ok(i64::from_ne_bytes(take(bytes)?)))
            .map(Array::I64)
            .map(Value::Array),
        F64_ARRAY => read_array(bytes, |bytes| Ok(f64::from_ne_bytes(take(bytes)?)))
            .map(Array::F64)
            .map(Value::Array),
        STRING_ARRAY => read_array(bytes, |bytes| {
            Ok(StringValue::from(read_text(bytes)?.to_owned()))
        })
        .map(Array::String)
        .map(Value::Array),
        _ => Err(invalid()),
    }
}

fn read_array<T>(
    bytes: &mut &[u8],
    read: impl Fn(&mut &[u8]) -> Result<T, NativeError>,
) -> Result<Vec<T>, NativeError> {
    let count = usize::from_ne_bytes(take(bytes)?);
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| fail(MemoryError::CollectionReservationFailed))?;
    for _ in 0..count {
        values.push(read(bytes)?);
    }
    Ok(values)
}

pub(super) fn fail(_: MemoryError) -> NativeError {
    NativeError::Native(Defect::from(LibraryDefect::ProviderCapacityFull))
}

fn invalid() -> NativeError {
    NativeError::Native(Defect::new("invalid prepared telemetry record"))
}
