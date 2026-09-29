use crate::JsonValue;
use serde::Serialize;
use std::io::{self, Write};
use std::num::NonZeroUsize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewlinePolicy {
    Lf,
    AcceptCrLf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlankLinePolicy {
    Reject,
    Skip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinalLinePolicy {
    RequireNewline,
    AcceptEof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NdjsonPolicy {
    pub maximum_line: NonZeroUsize,
    pub newline: NewlinePolicy,
    pub blank: BlankLinePolicy,
    pub final_line: FinalLinePolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NdjsonError {
    LineTooLong {
        maximum: usize,
        observed_at_least: usize,
    },
    LengthOverflow,
    BlankLine,
    InvalidCarriageReturn,
    TruncatedLine,
    InternalBounds,
}

impl std::fmt::Display for NdjsonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for NdjsonError {}

/// Completed lines remain owned by the caller when a later line in the same
/// input is invalid. The decoder cannot be reused after `error`.
#[derive(Debug)]
pub struct NdjsonDecodeFailure {
    pub completed: Vec<Vec<u8>>,
    pub error: NdjsonError,
}

#[derive(Debug)]
pub enum NdjsonEncodeError {
    Json(serde_json::Error),
    Framing(NdjsonError),
}

impl std::fmt::Display for NdjsonEncodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for NdjsonEncodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Framing(error) => Some(error),
        }
    }
}

/// Serialize one JSON value as one LF-terminated NDJSON record. The matching
/// schema codec remains authoritative for application value validation.
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

struct BoundedJsonWriter {
    bytes: Vec<u8>,
    maximum: usize,
    exceeded: Option<NdjsonError>,
}

impl Write for BoundedJsonWriter {
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
    pub fn new(policy: NdjsonPolicy) -> Self {
        Self {
            policy,
            pending: Vec::new(),
            poisoned: None,
        }
    }

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

    fn fail<T>(
        &mut self,
        error: NdjsonError,
        completed: Vec<Vec<u8>>,
    ) -> Result<T, NdjsonDecodeFailure> {
        self.poisoned = Some(error);
        Err(NdjsonDecodeFailure { completed, error })
    }
}
