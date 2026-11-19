//! Region-owned issue records, argument records and their shared text.

use effect::memory::{
    Buffer, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};
use effect::{IssueSegment, SchemaIssue, SchemaIssueKind};
use std::alloc::Layout;
use std::fmt::{self, Write as _};
use std::num::{NonZeroU8, NonZeroUsize};
use std::ops::Range;

use crate::byte_text::BufferWriter;

use super::contracts::with_issue_diagnostic;
use super::issues::{
    VALIDATION_ISSUE_SCOPE_FIELD, VALIDATION_ISSUE_SCOPE_FORM, VALIDATION_ISSUE_SOURCE_CUE,
};
use super::{ValidationArgument, ValidationIssue, ValidationResult};

/// Physical validation result slots and per-result policy.
#[derive(Clone, Copy)]
pub struct ValidationResultCapacity {
    /// Concurrent results, including results retained by their callers.
    pub results: NonZeroUsize,
    /// Number of issue records per result.
    pub issues: NonZeroUsize,
    /// Total number of translation arguments in one result.
    pub arguments: usize,
    /// Maximum bytes in one dotted issue path.
    pub path_bytes: Bytes,
    /// Combined bytes for result text, issues, paths and arguments.
    pub text_bytes: Bytes,
}

struct IssueRecord {
    text: [Range<usize>; 10],
    arguments: Range<usize>,
}

struct ArgumentRecord {
    name: Range<usize>,
    value: ArgumentValue,
}

enum ArgumentValue {
    Integer(i64),
    Number(f64),
    Text(Range<usize>),
}

struct ResultStorage {
    issues: FixedVector<'static, IssueRecord>,
    arguments: FixedVector<'static, ArgumentRecord>,
    text: Buffer<'static>,
}

/// Startup-provisioned validation outcome storage.
pub struct PreparedValidationResults {
    slots: SlotPool<'static, ResultStorage>,
    path_bytes: Bytes,
}

/// One checked-out publication cell, held before running validation.
pub struct ValidationResultReservation {
    storage: SlotLease<'static, ResultStorage>,
    path_bytes: Bytes,
}

/// A bounded validation result retaining its backing until the owner is released.
pub struct PreparedValidationResult {
    storage: SlotLease<'static, ResultStorage>,
    action: Range<usize>,
    code: Range<usize>,
    message: Range<usize>,
    /// Whether the presentation disables submission.
    pub submit_disabled: bool,
    /// Whether this invalid form needs rehydration.
    pub requires_rehydrate: bool,
}

/// Borrowed view over a retained validation issue.
pub struct PreparedValidationIssue<'a> {
    /// Issue code.
    pub code: &'a str,
    /// Issue source.
    pub source: &'a str,
    /// Issue scope.
    pub scope: &'a str,
    /// Dotted validation path.
    pub path: &'a str,
    /// Submitted field name.
    pub field: &'a str,
    /// Rendered field identifier.
    pub field_id: &'a str,
    /// Declared field key.
    pub field_key: &'a str,
    /// Declared section key.
    pub section_key: &'a str,
    /// Explicit translation key.
    pub message_key: &'a str,
    /// Original fallback message.
    pub message: &'a str,
    arguments: &'a [ArgumentRecord],
    bytes: &'a [u8],
}

/// Borrowed translation argument, preserving its native scalar kind.
pub enum PreparedValidationArgument<'a> {
    /// Integer.
    Integer(i64),
    /// Floating-point number.
    Number(f64),
    /// Text in the retained result.
    Text(&'a str),
}

impl PreparedValidationResults {
    /// Size the real slot records and their constituent storage.
    /// Allocator headers and alignment are accounted by the caller's memory plan.
    pub fn storage_bytes(capacity: ValidationResultCapacity) -> Result<Bytes, MemoryError> {
        let issues = Layout::array::<IssueRecord>(capacity.issues.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        let arguments = Layout::array::<ArgumentRecord>(capacity.arguments)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let payload = Bytes::new(issues.size())
            .checked_add(Bytes::new(arguments.size()))?
            .checked_add(capacity.text_bytes)?;
        SlotPool::<ResultStorage>::storage_bytes(capacity.results.get())?
            .checked_add(payload.checked_mul(capacity.results.get())?)
    }

    /// Prepare every record and byte buffer before accepting validation work.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: ValidationResultCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = Self::storage_bytes(capacity)?;
        let slots = memory.slots(region, capacity.results.get(), |_| {
            Ok(ResultStorage {
                issues: FixedVector::prepare(memory.allocator(region)?, capacity.issues.get())?,
                arguments: FixedVector::prepare(memory.allocator(region)?, capacity.arguments)?,
                text: memory.buffer(region, capacity.text_bytes)?,
            })
        })?;
        Ok(Self {
            slots,
            path_bytes: capacity.path_bytes,
        })
    }

    /// Take all physical publication backing together or refuse without work.
    pub fn reserve(&self) -> Result<ValidationResultReservation, MemoryError> {
        let mut storage = self.slots.try_checkout()?;
        storage.issues.clear();
        storage.arguments.clear();
        storage.text.clear();
        Ok(ValidationResultReservation {
            storage,
            path_bytes: self.path_bytes,
        })
    }
}

impl ValidationResultReservation {
    /// Publish an invalid contract result directly from the borrowed schema failure.
    /// Groups flatten in schema order. The result owns no intermediate issue vector.
    pub fn publish_schema_rejection(
        mut self,
        action: &str,
        code: &str,
        issue: &SchemaIssue,
        group_depth: NonZeroU8,
    ) -> Result<PreparedValidationResult, MemoryError> {
        let storage = &mut *self.storage;
        let action = append(&mut storage.text, action)?;
        let code = append(&mut storage.text, code)?;
        storage.push_schema_group(issue, self.path_bytes, group_depth.get())?;
        let message = storage
            .issues
            .as_slice()
            .first()
            .map_or(0..0, |issue| issue.text[9].clone());
        Ok(PreparedValidationResult {
            storage: self.storage,
            action,
            code,
            message,
            submit_disabled: true,
            requires_rehydrate: true,
        })
    }

    /// Publish the selected result in its reserved issue, argument and text backing.
    /// Failure publishes no partial result or silently omitted issue.
    pub fn publish(
        mut self,
        result: ValidationResult,
    ) -> Result<PreparedValidationResult, MemoryError> {
        if result.issues.len() > self.storage.issues.capacity() {
            return Err(MemoryError::NoSlot);
        }
        let issue_path_limit = self.path_bytes;
        let storage = &mut *self.storage;
        let action = append(&mut storage.text, result.action.as_str())?;
        let code = append(&mut storage.text, result.code.as_str())?;
        let message = append(&mut storage.text, &result.message)?;
        for issue in &result.issues {
            let path_bytes = Bytes::new(issue.path.len());
            if path_bytes > issue_path_limit {
                return Err(MemoryError::ValueTooLarge {
                    requested: path_bytes,
                    capacity: issue_path_limit,
                });
            }
            storage.push_issue(issue)?;
        }
        Ok(PreparedValidationResult {
            storage: self.storage,
            action,
            code,
            message,
            submit_disabled: result.submit_disabled,
            requires_rehydrate: result.requires_rehydrate,
        })
    }
}

impl ResultStorage {
    fn push_schema_group(
        &mut self,
        issue: &SchemaIssue,
        path_bytes: Bytes,
        group_depth: u8,
    ) -> Result<(), MemoryError> {
        if let SchemaIssueKind::Memory(error) = issue.kind() {
            return Err(*error);
        }
        if matches!(issue.kind(), SchemaIssueKind::Multiple) && !issue.children().is_empty() {
            let next = group_depth
                .checked_sub(1)
                .filter(|depth| *depth != 0)
                .ok_or(MemoryError::NoSlot)?;
            for child in issue.children() {
                self.push_schema_group(child, path_bytes, next)?;
            }
            Ok(())
        } else {
            self.push_schema_issue(issue, path_bytes)
        }
    }

    fn push_schema_issue(
        &mut self,
        issue: &SchemaIssue,
        path_bytes: Bytes,
    ) -> Result<(), MemoryError> {
        if self.issues.len() == self.issues.capacity() {
            return Err(MemoryError::NoSlot);
        }
        let path_start = self.text.len();
        let raw = append_schema_path(&mut self.text, issue)?;
        let path_len = Bytes::new(raw.len());
        if path_len > path_bytes {
            return Err(MemoryError::ValueTooLarge {
                requested: path_len,
                capacity: path_bytes,
            });
        }
        let raw = text_at(self.text.as_slice(), &raw);
        let scope = if raw.is_empty() {
            VALIDATION_ISSUE_SCOPE_FORM
        } else {
            VALIDATION_ISSUE_SCOPE_FIELD
        };
        let leading = raw.len() - raw.trim_start().len();
        let path = (path_start + leading)..(path_start + leading + raw.trim().len());
        let source = append(&mut self.text, VALIDATION_ISSUE_SOURCE_CUE)?;
        let scope = append(&mut self.text, scope)?;
        with_issue_diagnostic(issue.kind(), |code, message, lengths| {
            let code = append(&mut self.text, code)?;
            let message = append_fmt(&mut self.text, message)?;
            let raw_message = text_at(self.text.as_slice(), &message);
            let end = message.start + raw_message.trim_end().len();
            let start = end - raw_message.trim().len();
            let first = self.arguments.len();
            for (name, value) in lengths.into_iter().flatten() {
                let name = append(&mut self.text, name)?;
                let value = match i64::try_from(value) {
                    Ok(value) => ArgumentValue::Integer(value),
                    Err(_) => {
                        ArgumentValue::Text(append_fmt(&mut self.text, format_args!("{value}"))?)
                    }
                };
                self.arguments
                    .push(ArgumentRecord { name, value })
                    .map_err(|_| MemoryError::NoSlot)?;
            }
            let text = [
                code,
                source,
                scope,
                path.clone(),
                path.clone(),
                path,
                0..0,
                0..0,
                0..0,
                start..end,
            ];
            self.issues
                .push(IssueRecord {
                    text,
                    arguments: first..self.arguments.len(),
                })
                .map_err(|_| MemoryError::NoSlot)
        })
    }

    fn push_issue(&mut self, issue: &ValidationIssue) -> Result<(), MemoryError> {
        let text = [
            append(&mut self.text, issue.code.as_str())?,
            append(&mut self.text, issue.source.as_str())?,
            append(&mut self.text, issue.scope.as_str())?,
            append(&mut self.text, &issue.path)?,
            append(&mut self.text, &issue.field)?,
            append(&mut self.text, &issue.field_id)?,
            append(&mut self.text, &issue.field_key)?,
            append(&mut self.text, &issue.section_key)?,
            append(&mut self.text, &issue.message_key)?,
            append(&mut self.text, &issue.message)?,
        ];
        let start = self.arguments.len();
        for (name, value) in &issue.args {
            let name = append(&mut self.text, name)?;
            let value = match value {
                ValidationArgument::Integer(value) => ArgumentValue::Integer(*value),
                ValidationArgument::Number(value) => ArgumentValue::Number(*value),
                ValidationArgument::Text(value) => {
                    ArgumentValue::Text(append(&mut self.text, value)?)
                }
            };
            self.arguments
                .push(ArgumentRecord { name, value })
                .map_err(|_| MemoryError::NoSlot)?;
        }
        self.issues
            .push(IssueRecord {
                text,
                arguments: start..self.arguments.len(),
            })
            .map_err(|_| MemoryError::NoSlot)
    }
}

fn append_schema_path(
    output: &mut Buffer<'_>,
    issue: &SchemaIssue,
) -> Result<Range<usize>, MemoryError> {
    let start = output.len();
    let mut separated = false;
    for segment in issue.path() {
        if matches!(segment, IssueSegment::Alternative(_)) {
            continue;
        }
        if separated {
            output.extend_from_slice(b".")?;
        }
        if let Some(name) = segment.text() {
            output.extend_from_slice(name.as_bytes())?;
            separated = true;
        } else if let IssueSegment::Index(index) = segment {
            append_fmt(output, format_args!("{index}"))?;
            separated = true;
        }
    }
    let suffix = if let SchemaIssueKind::UnknownField(name) = issue.kind() {
        Some(name.as_ref())
    } else if let SchemaIssueKind::PreparedUnknownField(name) = issue.kind() {
        Some(name.as_str())
    } else {
        None
    };
    if let Some(suffix) = suffix {
        if output.len() != start {
            output.extend_from_slice(b".")?;
        }
        output.extend_from_slice(suffix.as_bytes())?;
    }
    Ok(start..output.len())
}

fn append_fmt(
    output: &mut Buffer<'_>,
    value: fmt::Arguments<'_>,
) -> Result<Range<usize>, MemoryError> {
    let start = output.len();
    let mut writer = BufferWriter::new(output);
    writer
        .write_fmt(value)
        .map_err(|_| writer.memory_error().unwrap_or(MemoryError::InvalidRange))?;
    Ok(start..output.len())
}

fn append(buffer: &mut Buffer<'_>, text: &str) -> Result<Range<usize>, MemoryError> {
    let start = buffer.len();
    buffer.extend_from_slice(text.as_bytes())?;
    Ok(start..buffer.len())
}

fn text_at<'a>(bytes: &'a [u8], range: &Range<usize>) -> &'a str {
    bytes
        .get(range.clone())
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or_default()
}

impl PreparedValidationResult {
    /// Borrow the exact action name.
    #[must_use]
    pub fn action(&self) -> &str {
        text_at(self.storage.text.as_slice(), &self.action)
    }

    /// Borrow the exact outcome code.
    #[must_use]
    pub fn code(&self) -> &str {
        text_at(self.storage.text.as_slice(), &self.code)
    }

    /// Borrow the exact form summary.
    #[must_use]
    pub fn message(&self) -> &str {
        text_at(self.storage.text.as_slice(), &self.message)
    }

    /// Read issues in their original publication order.
    pub fn issues(&self) -> impl Iterator<Item = PreparedValidationIssue<'_>> {
        let bytes = self.storage.text.as_slice();
        let arguments = self.storage.arguments.as_slice();
        self.storage.issues.as_slice().iter().map(move |issue| {
            let [
                code,
                source,
                scope,
                path,
                field,
                field_id,
                field_key,
                section_key,
                message_key,
                message,
            ] = &issue.text;
            PreparedValidationIssue {
                code: text_at(bytes, code),
                source: text_at(bytes, source),
                scope: text_at(bytes, scope),
                path: text_at(bytes, path),
                field: text_at(bytes, field),
                field_id: text_at(bytes, field_id),
                field_key: text_at(bytes, field_key),
                section_key: text_at(bytes, section_key),
                message_key: text_at(bytes, message_key),
                message: text_at(bytes, message),
                arguments: arguments.get(issue.arguments.clone()).unwrap_or_default(),
                bytes,
            }
        })
    }
}

impl PreparedValidationIssue<'_> {
    /// Borrow the issue's typed translation arguments in their original order.
    pub fn arguments(&self) -> impl Iterator<Item = (&str, PreparedValidationArgument<'_>)> {
        self.arguments.iter().map(|argument| {
            let value = match &argument.value {
                ArgumentValue::Integer(value) => PreparedValidationArgument::Integer(*value),
                ArgumentValue::Number(value) => PreparedValidationArgument::Number(*value),
                ArgumentValue::Text(value) => {
                    PreparedValidationArgument::Text(text_at(self.bytes, value))
                }
            };
            (text_at(self.bytes, &argument.name), value)
        })
    }
}
