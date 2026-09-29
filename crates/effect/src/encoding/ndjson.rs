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

/// Incremental line framing. It returns encoded JSON records as owned bytes;
/// decoding/validation remains the job of the caller's typed schema codec.
pub struct NdjsonDecoder {
    policy: NdjsonPolicy,
    pending: Vec<u8>,
    poisoned: Option<NdjsonError>,
}

impl NdjsonDecoder {
    #[must_use]
    /// Construct a `NdjsonDecoder` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `policy` (`NdjsonPolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Self`.
    pub fn new(policy: NdjsonPolicy) -> Self {
        Self {
            policy,
            pending: Vec::new(),
            poisoned: None,
        }
    }

    /// Append a value through this collection’s ordering and capacity rules.
    ///
    /// # Parameters
    /// - `input` (`&[u8]`): The input value or factory consumed by this operation.
    ///
    /// # Returns
    /// Type: `Result<Vec<Vec<u8>>, NdjsonDecodeFailure>`.
    pub fn push(&mut self, input: &[u8]) -> Result<Vec<Vec<u8>>, NdjsonDecodeFailure> {
        if let Some(error) = self.poisoned {
            return Err(NdjsonDecodeFailure {
                completed: Vec::new(),
                error,
            });
        }
        let mut complete = Vec::new();
        let mut start = 0;
        for (index, byte) in input.iter().enumerate() {
            if *byte != b'\n' {
                continue;
            }
            let Some(segment) = input.get(start..index) else {
                return self.fail(NdjsonError::InternalBounds, complete);
            };
            if let Err(error) = self.extend(segment) {
                return self.fail(error, complete);
            }
            if self.pending.last() == Some(&b'\r') {
                match self.policy.newline {
                    NewlinePolicy::Lf => {
                        return self.fail(NdjsonError::InvalidCarriageReturn, complete);
                    }
                    NewlinePolicy::AcceptCrLf => {
                        self.pending.pop();
                    }
                }
            }
            if self.pending.len() > self.policy.maximum_line.get() {
                return self.fail(
                    NdjsonError::LineTooLong {
                        maximum: self.policy.maximum_line.get(),
                        observed_at_least: self.pending.len(),
                    },
                    complete,
                );
            }
            if self.pending.is_empty() {
                if self.policy.blank == BlankLinePolicy::Reject {
                    return self.fail(NdjsonError::BlankLine, complete);
                }
            } else {
                complete.push(std::mem::take(&mut self.pending));
            }
            start = index + b"\n".len();
        }
        let Some(segment) = input.get(start..) else {
            return self.fail(NdjsonError::InternalBounds, complete);
        };
        if let Err(error) = self.extend(segment) {
            return self.fail(error, complete);
        }
        Ok(complete)
    }

    /// Finish one byte stream. `AcceptEof` transfers a final unterminated
    /// record; `RequireNewline` reports it as incomplete instead.
    ///
    /// # Returns
    /// Type: `Result<Option<Vec<u8>>, NdjsonError>`.
    pub fn finish(mut self) -> Result<Option<Vec<u8>>, NdjsonError> {
        if let Some(error) = self.poisoned {
            return Err(error);
        }
        if self.pending.is_empty() {
            return Ok(None);
        }
        if self.policy.final_line == FinalLinePolicy::RequireNewline {
            return Err(NdjsonError::TruncatedLine);
        }
        if self.pending.last() == Some(&b'\r') {
            return Err(NdjsonError::InvalidCarriageReturn);
        }
        if self.pending.len() > self.policy.maximum_line.get() {
            return Err(NdjsonError::LineTooLong {
                maximum: self.policy.maximum_line.get(),
                observed_at_least: self.pending.len(),
            });
        }
        Ok(Some(std::mem::take(&mut self.pending)))
    }

    ///
    /// # Parameters
    /// - `input` (`&[u8]`): Owned input decoded or consumed by this operation.
    ///
    /// # Returns
    /// Type: `Result<(), NdjsonError>`.
    fn extend(&mut self, input: &[u8]) -> Result<(), NdjsonError> {
        let length = self
            .pending
            .len()
            .checked_add(input.len())
            .ok_or(NdjsonError::LengthOverflow)?;
        let allowance = if self.policy.newline == NewlinePolicy::AcceptCrLf {
            self.policy
                .maximum_line
                .get()
                .checked_add(size_of::<u8>())
                .ok_or(NdjsonError::LengthOverflow)?
        } else {
            self.policy.maximum_line.get()
        };
        if length > allowance {
            return Err(NdjsonError::LineTooLong {
                maximum: self.policy.maximum_line.get(),
                observed_at_least: length,
            });
        }
        self.pending.extend_from_slice(input);
        Ok(())
    }

    ///
    /// # Type parameters
    /// - `T`: Stored or transferred value type.
    ///
    /// # Parameters
    /// - `error` (`NdjsonError`): Expected or provider error preserved in the returned outcome.
    /// - `completed` (`Vec<Vec<u8>>`): The completed supplied to this operation.
    ///
    /// # Returns
    /// Type: `Result<T, NdjsonDecodeFailure>`.
    fn fail<T>(
        &mut self,
        error: NdjsonError,
        completed: Vec<Vec<u8>>,
    ) -> Result<T, NdjsonDecodeFailure> {
        self.poisoned = Some(error);
        Err(NdjsonDecodeFailure { completed, error })
    }
}
