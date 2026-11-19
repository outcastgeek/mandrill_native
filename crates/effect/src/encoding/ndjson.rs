//! Newline-delimited JSON encoding and incremental parsing.

use crate::JsonValue;
use serde::Serialize;
use std::io::{self, Write};
use std::num::NonZeroUsize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The newline convention used when encoding newline-delimited JSON.
pub enum NewlinePolicy {
    /// Use LF as the line ending.
    Lf,
    /// Accept CRLF as a line ending.
    AcceptCrLf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// The treatment of blank lines in newline-delimited JSON input.
pub enum BlankLinePolicy {
    /// Reject the conflicting input without applying its proposed state change.
    Reject,
    /// Leave the occurrence or value unchanged and continue.
    Skip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Whether an unterminated last line is accepted as a complete record.
pub enum FinalLinePolicy {
    /// Reject a final line without a terminating newline.
    RequireNewline,
    /// Accept end-of-file as the final line boundary.
    AcceptEof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Framing and line-handling rules for newline-delimited JSON.
pub struct NdjsonPolicy {
    /// Largest accepted encoded line length. Type: `NonZeroUsize`.
    pub maximum_line: NonZeroUsize,
    /// Newline convention used during encoding. Type: `NewlinePolicy`.
    pub newline: NewlinePolicy,
    /// Policy for blank input lines. Type: `BlankLinePolicy`.
    pub blank: BlankLinePolicy,
    /// Treatment of a last line without a trailing newline. Type: `FinalLinePolicy`.
    pub final_line: FinalLinePolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A line-framing or resource-limit failure while reading NDJSON.
pub enum NdjsonError {
    /// The encoded line exceeds the configured size limit.
    LineTooLong {
        /// Largest value permitted by this policy. Type: `usize`.
        maximum: usize,
        /// Minimum line length observed before this limit was reported. Type: `usize`.
        observed_at_least: usize,
    },
    /// Computing length exceeded its supported range.
    LengthOverflow,
    /// A blank line is forbidden by the selected NDJSON policy.
    BlankLine,
    /// The carriage return failed validation.
    InvalidCarriageReturn,
    /// The final line ended before a complete value was available.
    TruncatedLine,
    /// The encoded length exceeded a checked internal bound.
    InternalBounds,
}

impl std::fmt::Display for NdjsonError {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for NdjsonError {}

/// Completed lines remain owned by the caller when a later line in the same
/// input is invalid. The decoder cannot be reused after `error`.
#[derive(Debug)]
pub struct NdjsonDecodeFailure {
    /// Results that have already reached a terminal state. Type: `Vec<Vec<u8>>`.
    pub completed: Vec<Vec<u8>>,
    /// The typed error carried by this result. Type: `NdjsonError`.
    pub error: NdjsonError,
}

#[derive(Debug)]
/// A JSON encoding or line-framing failure while writing NDJSON.
pub enum NdjsonEncodeError {
    /// JSON parsing or encoding failed. Retains the `serde_json::Error` detail.
    Json(serde_json::Error),
    /// The input does not satisfy the selected framing rules. Retains the `NdjsonError` detail.
    Framing(NdjsonError),
}

impl std::fmt::Display for NdjsonEncodeError {
    ///
    /// # Parameters
    /// - `formatter` (`&mut std::fmt::Formatter<'_>`): Destination formatter used by the Rust formatting trait.
    ///
    /// # Returns
    /// Type: `std::fmt::Result`.
    /// Returns `std::fmt::Result`.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for NdjsonEncodeError {
    ///
    /// # Returns
    /// Type: `Option<&(dyn std::error::Error + 'static)>`.
    /// Returns the value when present, or `None`.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Framing(error) => Some(error),
        }
    }
}

/// Serialize one JSON value as one LF-terminated NDJSON record. The matching
/// schema codec remains authoritative for application value validation.
///
/// # Parameters
/// - `value` (`&JsonValue`): Value read, transformed, encoded, or stored by this operation.
/// - `policy` (`NdjsonPolicy`): The caller-supplied behavior policy.
///
/// # Returns
/// Type: `Result<Vec<u8>, NdjsonEncodeError>`.
pub fn encode_ndjson(
    value: &JsonValue,
    policy: NdjsonPolicy,
) -> Result<Vec<u8>, NdjsonEncodeError> {
    let mut writer = BoundedJsonWriter {
        bytes: Vec::new(),
        maximum: policy.maximum_line.get(),
        exceeded: None,
    };
    let result = {
        let mut serializer = serde_json::Serializer::new(&mut writer);
        value.serialize(serde_stacker::Serializer::new(&mut serializer))
    };
    if let Err(error) = result {
        return match writer.exceeded {
            Some(error) => Err(NdjsonEncodeError::Framing(error)),
            None => Err(NdjsonEncodeError::Json(error)),
        };
    }
    writer.bytes.push(b'\n');
    Ok(writer.bytes)
}

/// Typed bounded json writer used by encoding ndjson.
struct BoundedJsonWriter {
    bytes: Vec<u8>,
    maximum: usize,
    exceeded: Option<NdjsonError>,
}

impl Write for BoundedJsonWriter {
    ///
    /// # Parameters
    /// - `bytes` (`&[u8]`): Encoded bytes to validate, decode, write, or transfer.
    ///
    /// # Returns
    /// Type: `io::Result<usize>`.
    /// Returns `io::Result<usize>`.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let attempted = self.bytes.len().checked_add(bytes.len());
        match attempted {
            Some(length) if length <= self.maximum => {
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            Some(length) => {
                self.exceeded = Some(NdjsonError::LineTooLong {
                    maximum: self.maximum,
                    observed_at_least: length,
                });
                Err(io::ErrorKind::Other.into())
            }
            None => {
                self.exceeded = Some(NdjsonError::LengthOverflow);
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

/// A framing failure or an actual prepared-memory refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedNdjsonError {
    /// Original NDJSON framing rejection.
    Framing(NdjsonError),
    /// Actual prepared storage refusal.
    Memory(crate::memory::MemoryError),
}
impl std::fmt::Display for PreparedNdjsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Framing(error) => error.fmt(f),
            Self::Memory(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for PreparedNdjsonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Framing(error) => Some(error),
            Self::Memory(error) => Some(error),
        }
    }
}
/// Completed payloads remain owned if a later line fails.
#[derive(Debug)]
pub struct PreparedNdjsonDecodeFailure {
    /// None means no result array was admitted and no input consumed: admission
    /// failed, or the decoder was already poisoned.
    pub completed: Option<crate::PreparedJsonArray<super::PreparedBytes>>,
    /// Exact framing or physical failure.
    pub error: PreparedNdjsonError,
}
trait LinePayload {
    fn len(&self) -> usize;
    fn last(&self) -> Option<u8>;
    fn pop(&mut self);
    fn append(&mut self, input: &[u8]) -> Result<(), crate::memory::MemoryError>;
}
impl LinePayload for Vec<u8> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn last(&self) -> Option<u8> {
        self.as_slice().last().copied()
    }
    fn pop(&mut self) {
        let _ = Vec::pop(self);
    }
    fn append(&mut self, input: &[u8]) -> Result<(), crate::memory::MemoryError> {
        self.extend_from_slice(input);
        Ok(())
    }
}
impl LinePayload for super::PreparedBytes {
    fn len(&self) -> usize {
        self.as_slice().len()
    }
    fn last(&self) -> Option<u8> {
        self.as_slice().last().copied()
    }
    fn pop(&mut self) {
        if let Some(buffer) = self.buffer_mut() {
            buffer.truncate(buffer.len().saturating_sub(1));
        }
    }
    fn append(&mut self, input: &[u8]) -> Result<(), crate::memory::MemoryError> {
        self.buffer_mut()
            .ok_or(crate::memory::MemoryError::NoSlot)?
            .extend_from_slice(input)
    }
}
trait LineOutput<P: LinePayload> {
    fn begin(&mut self) -> Result<P, crate::memory::MemoryError>;
    fn complete(&mut self, payload: P) -> Result<(), crate::memory::MemoryError>;
}
struct NdjsonCore<P> {
    policy: NdjsonPolicy,
    pending: Option<P>,
    poisoned: Option<PreparedNdjsonError>,
}
impl<P: LinePayload> NdjsonCore<P> {
    const fn new(policy: NdjsonPolicy) -> Self {
        Self {
            policy,
            pending: None,
            poisoned: None,
        }
    }
    fn fail<T>(&mut self, error: PreparedNdjsonError) -> Result<T, PreparedNdjsonError> {
        self.poisoned = Some(error);
        self.pending = None;
        Err(error)
    }
    fn extend(
        &mut self,
        input: &[u8],
        output: &mut impl LineOutput<P>,
    ) -> Result<(), PreparedNdjsonError> {
        let length = self
            .pending
            .as_ref()
            .map_or(0, LinePayload::len)
            .checked_add(input.len())
            .ok_or(PreparedNdjsonError::Framing(NdjsonError::LengthOverflow))?;
        let allowance = if self.policy.newline == NewlinePolicy::AcceptCrLf {
            self.policy
                .maximum_line
                .get()
                .checked_add(1)
                .ok_or(PreparedNdjsonError::Framing(NdjsonError::LengthOverflow))?
        } else {
            self.policy.maximum_line.get()
        };
        if length > allowance {
            return Err(PreparedNdjsonError::Framing(NdjsonError::LineTooLong {
                maximum: self.policy.maximum_line.get(),
                observed_at_least: length,
            }));
        }
        if !input.is_empty() {
            if self.pending.is_none() {
                self.pending = Some(output.begin().map_err(PreparedNdjsonError::Memory)?);
            }
            self.pending
                .as_mut()
                .ok_or(PreparedNdjsonError::Framing(NdjsonError::InternalBounds))?
                .append(input)
                .map_err(PreparedNdjsonError::Memory)?;
        }
        Ok(())
    }
    fn push(
        &mut self,
        input: &[u8],
        output: &mut impl LineOutput<P>,
    ) -> Result<(), PreparedNdjsonError> {
        if let Some(error) = self.poisoned {
            return Err(error);
        }
        let mut start = 0;
        for (index, byte) in input.iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            let Some(segment) = input.get(start..index) else {
                return self.fail(PreparedNdjsonError::Framing(NdjsonError::InternalBounds));
            };
            if let Err(error) = self.extend(segment, output) {
                return self.fail(error);
            }
            if self.pending.as_ref().and_then(LinePayload::last) == Some(b'\r') {
                match self.policy.newline {
                    NewlinePolicy::Lf => {
                        return self.fail(PreparedNdjsonError::Framing(
                            NdjsonError::InvalidCarriageReturn,
                        ));
                    }
                    NewlinePolicy::AcceptCrLf => {
                        if let Some(pending) = &mut self.pending {
                            pending.pop();
                        }
                    }
                }
            }
            let len = self.pending.as_ref().map_or(0, LinePayload::len);
            if len > self.policy.maximum_line.get() {
                return self.fail(PreparedNdjsonError::Framing(NdjsonError::LineTooLong {
                    maximum: self.policy.maximum_line.get(),
                    observed_at_least: len,
                }));
            }
            if len == 0 {
                if self.policy.blank == BlankLinePolicy::Reject {
                    return self.fail(PreparedNdjsonError::Framing(NdjsonError::BlankLine));
                }
            } else if let Some(payload) = self.pending.take()
                && let Err(error) = output.complete(payload)
            {
                return self.fail(PreparedNdjsonError::Memory(error));
            }
            start = index + 1;
        }
        let Some(segment) = input.get(start..) else {
            return self.fail(PreparedNdjsonError::Framing(NdjsonError::InternalBounds));
        };
        if let Err(error) = self.extend(segment, output) {
            return self.fail(error);
        }
        Ok(())
    }
    fn finish(mut self) -> Result<Option<P>, PreparedNdjsonError> {
        if let Some(error) = self.poisoned {
            return Err(error);
        }
        let len = self.pending.as_ref().map_or(0, LinePayload::len);
        if len == 0 {
            return Ok(None);
        }
        if self.policy.final_line == FinalLinePolicy::RequireNewline {
            return Err(PreparedNdjsonError::Framing(NdjsonError::TruncatedLine));
        }
        if self.pending.as_ref().and_then(LinePayload::last) == Some(b'\r') {
            return Err(PreparedNdjsonError::Framing(
                NdjsonError::InvalidCarriageReturn,
            ));
        }
        if len > self.policy.maximum_line.get() {
            return Err(PreparedNdjsonError::Framing(NdjsonError::LineTooLong {
                maximum: self.policy.maximum_line.get(),
                observed_at_least: len,
            }));
        }
        Ok(self.pending.take())
    }
}
struct OrdinaryLines {
    complete: Vec<Vec<u8>>,
}
impl LineOutput<Vec<u8>> for OrdinaryLines {
    fn begin(&mut self) -> Result<Vec<u8>, crate::memory::MemoryError> {
        Ok(Vec::new())
    }
    fn complete(&mut self, value: Vec<u8>) -> Result<(), crate::memory::MemoryError> {
        self.complete.push(value);
        Ok(())
    }
}
/// Incremental line framing using ordinary owned bytes.
pub struct NdjsonDecoder {
    core: NdjsonCore<Vec<u8>>,
}
impl NdjsonDecoder {
    /// Construct a decoder with the supplied policy.
    #[must_use]
    pub const fn new(policy: NdjsonPolicy) -> Self {
        Self {
            core: NdjsonCore::new(policy),
        }
    }
    /// Consume a chunk, retaining completed lines on failure.
    pub fn push(&mut self, input: &[u8]) -> Result<Vec<Vec<u8>>, NdjsonDecodeFailure> {
        let mut output = OrdinaryLines {
            complete: Vec::new(),
        };
        match self.core.push(input, &mut output) {
            Ok(()) => Ok(output.complete),
            Err(error) => Err(NdjsonDecodeFailure {
                completed: output.complete,
                error: match error {
                    PreparedNdjsonError::Framing(error) => error,
                    PreparedNdjsonError::Memory(_) => NdjsonError::InternalBounds,
                },
            }),
        }
    }
    /// Finish a byte stream using the configured final-line policy.
    pub fn finish(self) -> Result<Option<Vec<u8>>, NdjsonError> {
        self.core.finish().map_err(|error| match error {
            PreparedNdjsonError::Framing(error) => error,
            PreparedNdjsonError::Memory(_) => NdjsonError::InternalBounds,
        })
    }
}
struct PreparedLines<'a> {
    pool: &'a super::PreparedBytePool,
    complete: &'a mut crate::PreparedJsonArray<super::PreparedBytes>,
}
impl LineOutput<super::PreparedBytes> for PreparedLines<'_> {
    fn begin(&mut self) -> Result<super::PreparedBytes, crate::memory::MemoryError> {
        self.pool.checkout()
    }
    fn complete(&mut self, value: super::PreparedBytes) -> Result<(), crate::memory::MemoryError> {
        self.complete
            .push(value)
            .map_err(|_| crate::memory::MemoryError::NoSlot)
    }
}
/// Prepared incremental line framing with actual retained payload cells.
///
/// A payload cell needs `maximum_line` bytes with LF-only framing. With
/// [`NewlinePolicy::AcceptCrLf`], it needs `maximum_line + 1` bytes for the
/// retained CR before normalization; that addition is checked and overflow
/// returns [`NdjsonError::LengthOverflow`]. Completed-result array storage
/// is admitted separately from the live payload cells.
pub struct PreparedNdjsonDecoder {
    core: NdjsonCore<super::PreparedBytes>,
    pool: super::PreparedBytePool,
}
impl PreparedNdjsonDecoder {
    /// Retain an existing prepared payload pool.
    pub fn prepare(
        policy: NdjsonPolicy,
        pool: &super::PreparedBytePool,
    ) -> Result<Self, crate::memory::MemoryError> {
        Ok(Self {
            core: NdjsonCore::new(policy),
            pool: pool.try_clone()?,
        })
    }
    /// Admit result storage first; a mid-chunk physical refusal poisons the decoder.
    pub fn push(
        &mut self,
        input: &[u8],
        results: &crate::PreparedJsonArrayPool<super::PreparedBytes>,
    ) -> Result<crate::PreparedJsonArray<super::PreparedBytes>, PreparedNdjsonDecodeFailure> {
        if let Some(error) = self.core.poisoned {
            return Err(PreparedNdjsonDecodeFailure {
                completed: None,
                error,
            });
        }
        let mut complete = results
            .reserve()
            .map_err(|error| PreparedNdjsonDecodeFailure {
                completed: None,
                error: PreparedNdjsonError::Memory(error),
            })?;
        let mut output = PreparedLines {
            pool: &self.pool,
            complete: &mut complete,
        };
        match self.core.push(input, &mut output) {
            Ok(()) => Ok(complete),
            Err(error) => Err(PreparedNdjsonDecodeFailure {
                completed: Some(complete),
                error,
            }),
        }
    }
    /// Finish and transfer an allowed final unterminated line.
    pub fn finish(self) -> Result<Option<super::PreparedBytes>, PreparedNdjsonError> {
        self.core.finish()
    }
}

/// A managed NDJSON encode failure preserving its actual domain.
#[derive(Debug)]
pub enum PreparedNdjsonEncodeError {
    /// Line-length or arithmetic rejection.
    Framing(NdjsonError),
    /// Actual prepared-memory refusal.
    Memory(crate::memory::MemoryError),
    /// Other prepared writer or representation failure.
    Schema(crate::SchemaIssue),
}
impl std::fmt::Display for PreparedNdjsonEncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Framing(error) => error.fmt(f),
            Self::Memory(error) => error.fmt(f),
            Self::Schema(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for PreparedNdjsonEncodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Framing(error) => Some(error),
            Self::Memory(error) => Some(error),
            Self::Schema(error) => Some(error),
        }
    }
}
fn prepared_encode_issue(issue: crate::SchemaIssue) -> PreparedNdjsonEncodeError {
    if let crate::SchemaIssueKind::Memory(error) = issue.kind() {
        PreparedNdjsonEncodeError::Memory(*error)
    } else {
        PreparedNdjsonEncodeError::Schema(issue)
    }
}
/// Append a managed JSON record and LF to a fixed buffer transactionally.
/// The writer's actual chunking determines the observed lower bound on a line-size failure.
pub fn encode_prepared_ndjson_into(
    value: &crate::SharedPreparedJsonValue,
    output: &mut crate::memory::Buffer<'_>,
    writers: &crate::PreparedJsonWriterPool,
    policy: NdjsonPolicy,
    depth: NonZeroUsize,
) -> Result<(), PreparedNdjsonEncodeError> {
    let mut writer = writers
        .admit_serde_buffered(crate::JsonWriterPolicy {
            bytes: policy.maximum_line,
            depth,
        })
        .map_err(PreparedNdjsonEncodeError::Memory)?;
    if let Err(issue) = value.view().write_to(&mut writer) {
        if matches!(
            issue.kind(),
            crate::SchemaIssueKind::LimitExceeded {
                resource: crate::SchemaResource::Bytes
            }
        ) && let Some(attempted) = writer.byte_limit_attempt()
        {
            return Err(PreparedNdjsonEncodeError::Framing(match attempted {
                Some(observed_at_least) => NdjsonError::LineTooLong {
                    maximum: policy.maximum_line.get(),
                    observed_at_least,
                },
                None => NdjsonError::LengthOverflow,
            }));
        }
        return Err(prepared_encode_issue(issue));
    }
    let bytes = writer.finish_prepared().map_err(prepared_encode_issue)?;
    let total = output
        .len()
        .checked_add(bytes.as_bytes().len())
        .and_then(|n| n.checked_add(1))
        .ok_or(PreparedNdjsonEncodeError::Framing(
            NdjsonError::LengthOverflow,
        ))?;
    if total > output.capacity().get() {
        return Err(PreparedNdjsonEncodeError::Memory(
            crate::memory::MemoryError::ValueTooLarge {
                requested: crate::memory::Bytes::new(total),
                capacity: output.capacity(),
            },
        ));
    }
    let start = output.len();
    let mut rollback =
        std::mem::DropGuard::new((output, start), |(output, start)| output.truncate(start));
    rollback
        .0
        .extend_from_slice(bytes.as_bytes())
        .map_err(PreparedNdjsonEncodeError::Memory)?;
    rollback
        .0
        .extend_from_slice(b"\n")
        .map_err(PreparedNdjsonEncodeError::Memory)?;
    let _ = std::mem::DropGuard::dismiss(rollback);
    Ok(())
}
