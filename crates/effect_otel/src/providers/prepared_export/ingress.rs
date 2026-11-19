//! Borrowed checks before the native batch processors take record ownership.

use super::{PreparedExport, Signal};
use effect::memory::Bytes;
use opentelemetry::logs::{AnyValue, Severity};
use opentelemetry::trace::{SpanContext, Status};
use opentelemetry::{Array, Context, InstrumentationScope, KeyValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{BatchLogProcessor, LogProcessor, SdkLogRecord};
use opentelemetry_sdk::trace::{BatchSpanProcessor, Span, SpanData, SpanProcessor};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

pub(super) struct CheckedProcessor<P> {
    pub(super) processor: P,
    pub(super) export: Arc<PreparedExport>,
    pub(super) record_bytes: Bytes,
    pub(super) log_depth: u8,
}

impl<P> fmt::Debug for CheckedProcessor<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CheckedProcessor")
            .field("record_bytes", &self.record_bytes)
            .finish_non_exhaustive()
    }
}

impl SpanProcessor for CheckedProcessor<BatchSpanProcessor> {
    fn on_start(&self, span: &mut Span, parent: &Context) {
        self.processor.on_start(span, parent);
    }

    fn on_end(&self, span: SpanData) {
        if Remaining(self.record_bytes.get()).span(&span) {
            self.processor.on_end(span);
        } else {
            self.export.discard_record(Signal::Traces);
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.processor.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.processor.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.processor.set_resource(resource);
    }
}

impl LogProcessor for CheckedProcessor<BatchLogProcessor> {
    fn emit(&self, record: &mut SdkLogRecord, scope: &InstrumentationScope) {
        if Remaining(self.record_bytes.get()).log(record, scope, self.log_depth) {
            self.processor.emit(record, scope);
        } else {
            self.export.discard_record(Signal::Logs);
        }
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.processor.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.processor.shutdown_with_timeout(timeout)
    }

    fn event_enabled(&self, level: Severity, target: &str, name: Option<&str>) -> bool {
        self.processor.event_enabled(level, target, name)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.processor.set_resource(resource);
    }
}

/// Count inline records, visible vector reserves, and exposed text/payload
/// lengths. Private SDK allocations and allocator rounding are external.
struct Remaining(usize);

impl Remaining {
    fn charge(&mut self, bytes: usize) -> bool {
        let Some(remaining) = self.0.checked_sub(bytes) else {
            return false;
        };
        self.0 = remaining;
        true
    }

    fn items<T>(&mut self, values: &Vec<T>) -> bool {
        values
            .capacity()
            .checked_mul(std::mem::size_of::<T>())
            .is_some_and(|bytes| self.charge(bytes))
    }

    fn text(&mut self, text: Option<&str>) -> bool {
        self.charge(text.map_or(0, str::len))
    }

    fn context(&mut self, context: &SpanContext) -> bool {
        context.trace_state().into_iter().all(|(name, value)| {
            self.charge(std::mem::size_of::<(String, String)>())
                && self.charge(name.len())
                && self.charge(value.len())
        })
    }

    fn attribute(&mut self, attribute: &KeyValue) -> bool {
        self.charge(attribute.key.as_str().len())
            && match &attribute.value {
                Value::Bool(_) | Value::I64(_) | Value::F64(_) => true,
                Value::String(value) => self.charge(value.as_str().len()),
                Value::Array(values) => match values {
                    Array::Bool(values) => self.items(values),
                    Array::I64(values) => self.items(values),
                    Array::F64(values) => self.items(values),
                    Array::String(values) => {
                        self.items(values)
                            && values.iter().all(|value| self.charge(value.as_str().len()))
                    }
                    _ => false,
                },
                _ => false,
            }
    }

    fn attributes(&mut self, attributes: &Vec<KeyValue>) -> bool {
        self.items(attributes) && attributes.iter().all(|attribute| self.attribute(attribute))
    }

    fn scope(&mut self, scope: &InstrumentationScope) -> bool {
        self.charge(scope.name().len())
            && self.text(scope.version())
            && self.text(scope.schema_url())
            && scope.attributes().all(|attribute| {
                self.charge(std::mem::size_of::<KeyValue>()) && self.attribute(attribute)
            })
    }

    fn span(&mut self, span: &SpanData) -> bool {
        let description = match &span.status {
            Status::Unset | Status::Ok => None,
            Status::Error { description } => Some(description.as_ref()),
        };
        self.charge(std::mem::size_of::<SpanData>())
            && self.charge(span.name.len())
            && self.text(description)
            && self.context(&span.span_context)
            && self.scope(&span.instrumentation_scope)
            && self.attributes(&span.attributes)
            && self.items(&span.events.events)
            && span
                .events
                .iter()
                .all(|event| self.charge(event.name.len()) && self.attributes(&event.attributes))
            && self.items(&span.links.links)
            && span
                .links
                .iter()
                .all(|link| self.context(&link.span_context) && self.attributes(&link.attributes))
    }

    fn log(&mut self, record: &SdkLogRecord, scope: &InstrumentationScope, depth: u8) -> bool {
        self.charge(std::mem::size_of::<SdkLogRecord>())
            && self.scope(scope)
            && self.text(record.event_name())
            && self.text(record.target().map(AsRef::as_ref))
            && self.text(record.severity_text())
            && record
                .body()
                .is_none_or(|value| self.log_value(value, depth))
            && record.attributes_iter().all(|(key, value)| {
                self.charge(std::mem::size_of::<(opentelemetry::Key, AnyValue)>())
                    && self.charge(key.as_str().len())
                    && self.log_value(value, depth)
            })
    }

    fn log_value(&mut self, value: &AnyValue, depth: u8) -> bool {
        match value {
            AnyValue::Boolean(_) | AnyValue::Int(_) | AnyValue::Double(_) => true,
            AnyValue::String(value) => self.charge(value.as_str().len()),
            AnyValue::Bytes(values) => self.items(values),
            AnyValue::ListAny(values) => depth.checked_sub(1).is_some_and(|depth| {
                self.items(values) && values.iter().all(|child| self.log_value(child, depth))
            }),
            AnyValue::Map(values) => depth.checked_sub(1).is_some_and(|depth| {
                values
                    .capacity()
                    .checked_mul(std::mem::size_of::<(opentelemetry::Key, AnyValue)>())
                    .is_some_and(|bytes| self.charge(bytes))
                    && values.iter().all(|(key, child)| {
                        self.charge(key.as_str().len()) && self.log_value(child, depth)
                    })
            }),
            // The SDK's value types are non-exhaustive.
            _ => false,
        }
    }
}
