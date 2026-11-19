//! Bounded incremental envelope framing.

use std::num::NonZeroUsize;

/// Format 1: one version octet, one unsigned 64-bit big-endian payload length,
/// then exactly that many payload octets. Versioning belongs to this format,
/// not to an application's schema, request identity or domain revision.
const VERSION: u8 = 1;
/// Number of bytes in the fixed frame header.
pub const FRAME_HEADER_LENGTH: usize = size_of::<u8>() + size_of::<u64>();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Caller-selected maximum frame size and framing behavior.
pub struct FramePolicy {
    /// The payload for maximum. Type: `NonZeroUsize`.
    pub maximum_payload: NonZeroUsize,
}

fn payload_length(header: &[u8], policy: FramePolicy) -> Result<usize, FrameError> {
    let (&version, length) = header.split_first().ok_or(FrameError::InternalBounds)?;
    if version != VERSION {
        return Err(FrameError::UnsupportedVersion(version));
    }
    let length = length.try_into().map_err(|_| FrameError::InternalBounds)?;
    let expected = usize::try_from(u64::from_be_bytes(length))
        .map_err(|_| FrameError::LengthOverflow)?;
    if expected > policy.maximum_payload.get() {
        return Err(FrameError::PayloadTooLarge {
            maximum: policy.maximum_payload.get(),
            actual: expected,
        });
    }
    Ok(expected)
}

pub(crate) enum CompleteFrames<'a> {
    Missing,
    One(&'a [u8]),
    Extra,
}

/// Validate complete input using the same framing rules without copying payloads.
/// A malformed later frame takes precedence over excess valid frames.
pub(crate) fn complete_frames(
    mut input: &[u8],
    policy: FramePolicy,
) -> Result<CompleteFrames<'_>, FrameError> {
    let mut frames = CompleteFrames::Missing;
    while !input.is_empty() {
        let (header, rest) = input.split_at_checked(FRAME_HEADER_LENGTH).ok_or(
            FrameError::Truncated { received: input.len(), expected: FRAME_HEADER_LENGTH },
        )?;
        let length = payload_length(header, policy)?;
        let (payload, next) = rest.split_at_checked(length).ok_or(FrameError::Truncated {
            received: rest.len(), expected: length,
        })?;
        frames = match frames {
            CompleteFrames::Missing => CompleteFrames::One(payload),
            CompleteFrames::One(_) | CompleteFrames::Extra => CompleteFrames::Extra,
        };
        input = next;
    }
    Ok(frames)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// A typed rejection of malformed or oversized framed input.
pub enum FrameError {
    /// The version is not supported. Carries `u8`.
    UnsupportedVersion(u8),
    /// Computing length exceeded its supported range.
    LengthOverflow,
    /// The frame payload exceeds the configured byte limit.
    PayloadTooLarge {
        /// Maximum permitted payload length. Type: `usize`.
        maximum: usize,
        /// Observed payload length. Type: `usize`.
        actual: usize,
    },
    /// The frame ended before all declared bytes arrived.
    Truncated {
        /// Bytes received so far. Type: `usize`.
        received: usize,
        /// Bytes required by the frame header. Type: `usize`.
        expected: usize,
    },
    /// The encoded length exceeded a checked internal bound.
    InternalBounds,
}

impl std::fmt::Display for FrameError {
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

impl std::error::Error for FrameError {}

/// Already accepted frames remain owned by the caller when a later frame in
/// the same input chunk is invalid. The decoder is poisoned after `error`.
#[derive(Debug)]
pub struct FrameDecodeFailure {
    /// Results that have already reached a terminal state. Type: `Vec<Vec<u8>>`.
    pub completed: Vec<Vec<u8>>,
    /// The typed error carried by this result. Type: `FrameError`.
    pub error: FrameError,
}

/// Scatter/gather representation avoids copying a payload merely to prepend
/// its header. `into_bytes` materializes a contiguous allocation on demand.
pub struct FrameParts {
    header: [u8; FRAME_HEADER_LENGTH],
    payload: Vec<u8>,
}

impl FrameParts {
    /// Construct a `FrameParts` from its required inputs and explicit policies.
    ///
    /// # Parameters
    /// - `payload` (`Vec<u8>`): Application data moved into the message or stored record.
    /// - `policy` (`FramePolicy`): The caller-supplied behavior policy.
    ///
    /// # Returns
    /// Type: `Result<Self, FrameError>`.
    pub fn new(payload: Vec<u8>, policy: FramePolicy) -> Result<Self, FrameError> {
        if payload.len() > policy.maximum_payload.get() {
            return Err(FrameError::PayloadTooLarge {
                maximum: policy.maximum_payload.get(),
                actual: payload.len(),
            });
        }
        let length = u64::try_from(payload.len()).map_err(|_| FrameError::LengthOverflow)?;
        let mut header = [0; FRAME_HEADER_LENGTH];
        header[0] = VERSION;
        header
            .get_mut(size_of::<u8>()..)
            .ok_or(FrameError::InternalBounds)?
            .copy_from_slice(&length.to_be_bytes());
        Ok(Self { header, payload })
    }

    #[must_use]
    /// Borrow the ordered segments of this path or identity.
    ///
    /// # Returns
    /// Type: `(&[u8], &[u8])`.
    pub fn segments(&self) -> (&[u8], &[u8]) {
        (&self.header, &self.payload)
    }

    /// Transfer the owned encoded bytes to the caller.
    ///
    /// # Returns
    /// Type: `Result<Vec<u8>, FrameError>`.
    pub fn into_bytes(mut self) -> Result<Vec<u8>, FrameError> {
        let total = FRAME_HEADER_LENGTH
            .checked_add(self.payload.len())
            .ok_or(FrameError::LengthOverflow)?;
        let mut bytes = Vec::with_capacity(total);
        bytes.extend_from_slice(&self.header);
        bytes.append(&mut self.payload);
        Ok(bytes)
    }
}

/// A protocol failure or a physical prepared-memory refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparedFrameError {
    /// Original frame protocol rejection.
    Framing(FrameError),
    /// Actual prepared storage refusal.
    Memory(crate::memory::MemoryError),
}
impl std::fmt::Display for PreparedFrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Framing(error) => error.fmt(f),
            Self::Memory(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for PreparedFrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Framing(error) => Some(error),
            Self::Memory(error) => Some(error),
        }
    }
}
/// Completed payloads remain owned even when a later frame fails.
#[derive(Debug)]
pub struct PreparedFrameDecodeFailure {
    /// None means no result array was admitted and no input consumed: admission
    /// failed, or the decoder was already poisoned.
    pub completed: Option<crate::PreparedJsonArray<super::PreparedBytes>>,
    /// Exact protocol or physical failure.
    pub error: PreparedFrameError,
}
trait Payload {
    fn len(&self) -> usize;
    fn append(&mut self, bytes: &[u8]) -> Result<(), crate::memory::MemoryError>;
}
impl Payload for Vec<u8> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn append(&mut self, bytes: &[u8]) -> Result<(), crate::memory::MemoryError> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}
impl Payload for super::PreparedBytes {
    fn len(&self) -> usize {
        self.as_slice().len()
    }
    fn append(&mut self, bytes: &[u8]) -> Result<(), crate::memory::MemoryError> {
        self.buffer_mut()
            .ok_or(crate::memory::MemoryError::NoSlot)?
            .extend_from_slice(bytes)
    }
}
trait FrameOutput<P: Payload> {
    fn begin(&mut self, expected: usize) -> Result<P, crate::memory::MemoryError>;
    fn empty(&mut self) -> Result<(), crate::memory::MemoryError>;
    fn complete(&mut self, payload: P) -> Result<(), crate::memory::MemoryError>;
}
enum DecodeState<P> {
    Header {
        bytes: [u8; FRAME_HEADER_LENGTH],
        used: usize,
    },
    Payload {
        expected: usize,
        bytes: P,
    },
    Poisoned(PreparedFrameError),
}
struct FrameCore<P> {
    policy: FramePolicy,
    state: DecodeState<P>,
}
impl<P: Payload> FrameCore<P> {
    const fn new(policy: FramePolicy) -> Self {
        Self {
            policy,
            state: DecodeState::Header {
                bytes: [0; FRAME_HEADER_LENGTH],
                used: 0,
            },
        }
    }
    fn poison(&mut self, error: PreparedFrameError) -> PreparedFrameError {
        self.state = DecodeState::Poisoned(error);
        error
    }
    fn push(
        &mut self,
        mut input: &[u8],
        output: &mut impl FrameOutput<P>,
    ) -> Result<(), PreparedFrameError> {
        if let DecodeState::Poisoned(error) = self.state {
            return Err(error);
        }
        while !input.is_empty() {
            let state = std::mem::replace(
                &mut self.state,
                DecodeState::Header {
                    bytes: [0; FRAME_HEADER_LENGTH],
                    used: 0,
                },
            );
            match state {
                DecodeState::Header {
                    mut bytes,
                    mut used,
                } => {
                    let take = (FRAME_HEADER_LENGTH - used).min(input.len());
                    let Some(target) = bytes.get_mut(used..used + take) else {
                        return Err(
                            self.poison(PreparedFrameError::Framing(FrameError::InternalBounds))
                        );
                    };
                    let (head, rest) = input.split_at(take);
                    target.copy_from_slice(head);
                    used += take;
                    input = rest;
                    if used == FRAME_HEADER_LENGTH {
                        let expected = match payload_length(&bytes, self.policy) {
                            Ok(value) => value,
                            Err(error) => return Err(self.poison(PreparedFrameError::Framing(error))),
                        };
                        if expected == 0 {
                            if let Err(error) = output.empty() {
                                return Err(self.poison(PreparedFrameError::Memory(error)));
                            }
                        } else {
                            match output.begin(expected) {
                                Ok(bytes) => self.state = DecodeState::Payload { expected, bytes },
                                Err(error) => {
                                    return Err(self.poison(PreparedFrameError::Memory(error)));
                                }
                            }
                        }
                    } else {
                        self.state = DecodeState::Header { bytes, used };
                    }
                }
                DecodeState::Payload {
                    expected,
                    mut bytes,
                } => {
                    let take = (expected - bytes.len()).min(input.len());
                    let (head, rest) = input.split_at(take);
                    if let Err(error) = bytes.append(head) {
                        return Err(self.poison(PreparedFrameError::Memory(error)));
                    }
                    input = rest;
                    if bytes.len() == expected {
                        if let Err(error) = output.complete(bytes) {
                            return Err(self.poison(PreparedFrameError::Memory(error)));
                        }
                    } else {
                        self.state = DecodeState::Payload { expected, bytes };
                    }
                }
                DecodeState::Poisoned(error) => {
                    self.state = DecodeState::Poisoned(error);
                    return Err(error);
                }
            }
        }
        Ok(())
    }
    fn finish(&self) -> Result<(), PreparedFrameError> {
        match &self.state {
            DecodeState::Header { used: 0, .. } => Ok(()),
            DecodeState::Header { used, .. } => {
                Err(PreparedFrameError::Framing(FrameError::Truncated {
                    received: *used,
                    expected: FRAME_HEADER_LENGTH,
                }))
            }
            DecodeState::Payload { expected, bytes } => {
                Err(PreparedFrameError::Framing(FrameError::Truncated {
                    received: bytes.len(),
                    expected: *expected,
                }))
            }
            DecodeState::Poisoned(error) => Err(*error),
        }
    }
}
struct OrdinaryOutput {
    complete: Vec<Vec<u8>>,
}
impl FrameOutput<Vec<u8>> for OrdinaryOutput {
    fn begin(&mut self, _: usize) -> Result<Vec<u8>, crate::memory::MemoryError> {
        Ok(Vec::new())
    }
    fn empty(&mut self) -> Result<(), crate::memory::MemoryError> {
        self.complete.push(Vec::new());
        Ok(())
    }
    fn complete(&mut self, payload: Vec<u8>) -> Result<(), crate::memory::MemoryError> {
        self.complete.push(payload);
        Ok(())
    }
}
/// Incremental decoder retaining at most one bounded in-progress payload.
pub struct FrameDecoder {
    core: FrameCore<Vec<u8>>,
}
impl FrameDecoder {
    /// Construct an ordinary decoder under the supplied policy.
    #[must_use]
    pub const fn new(policy: FramePolicy) -> Self {
        Self {
            core: FrameCore::new(policy),
        }
    }
    /// Consume a chunk, retaining completed frames on failure.
    pub fn push(&mut self, input: &[u8]) -> Result<Vec<Vec<u8>>, FrameDecodeFailure> {
        let mut output = OrdinaryOutput {
            complete: Vec::new(),
        };
        match self.core.push(input, &mut output) {
            Ok(()) => Ok(output.complete),
            Err(PreparedFrameError::Framing(error)) => Err(FrameDecodeFailure {
                completed: output.complete,
                error,
            }),
            Err(PreparedFrameError::Memory(_)) => Err(FrameDecodeFailure {
                completed: output.complete,
                error: FrameError::InternalBounds,
            }),
        }
    }
    /// Finish a stream and report a truncated frame when present.
    pub fn finish(&self) -> Result<(), FrameError> {
        self.core.finish().map_err(|error| match error {
            PreparedFrameError::Framing(error) => error,
            PreparedFrameError::Memory(_) => FrameError::InternalBounds,
        })
    }
}
struct PreparedOutput<'a> {
    pool: &'a super::PreparedBytePool,
    complete: &'a mut crate::PreparedJsonArray<super::PreparedBytes>,
}
impl FrameOutput<super::PreparedBytes> for PreparedOutput<'_> {
    fn begin(
        &mut self,
        expected: usize,
    ) -> Result<super::PreparedBytes, crate::memory::MemoryError> {
        let mut bytes = self.pool.checkout()?;
        let capacity = bytes
            .buffer_mut()
            .ok_or(crate::memory::MemoryError::NoSlot)?
            .capacity();
        if expected > capacity.get() {
            return Err(crate::memory::MemoryError::ValueTooLarge {
                requested: crate::memory::Bytes::new(expected),
                capacity,
            });
        }
        Ok(bytes)
    }
    fn empty(&mut self) -> Result<(), crate::memory::MemoryError> {
        self.complete
            .push(super::PreparedBytes::empty())
            .map_err(|_| crate::memory::MemoryError::NoSlot)
    }
    fn complete(
        &mut self,
        payload: super::PreparedBytes,
    ) -> Result<(), crate::memory::MemoryError> {
        self.complete
            .push(payload)
            .map_err(|_| crate::memory::MemoryError::NoSlot)
    }
}
/// Prepared decoder whose payloads and completed arrays retain real backing.
pub struct PreparedFrameDecoder {
    core: FrameCore<super::PreparedBytes>,
    pool: super::PreparedBytePool,
}
impl PreparedFrameDecoder {
    /// Retain an existing pool before exposing the decoder.
    pub fn prepare(
        policy: FramePolicy,
        pool: &super::PreparedBytePool,
    ) -> Result<Self, crate::memory::MemoryError> {
        Ok(Self {
            core: FrameCore::new(policy),
            pool: pool.try_clone()?,
        })
    }
    /// Admit a completed-result array before processing any bytes. Mid-chunk physical refusal is terminal.
    pub fn push(
        &mut self,
        input: &[u8],
        results: &crate::PreparedJsonArrayPool<super::PreparedBytes>,
    ) -> Result<crate::PreparedJsonArray<super::PreparedBytes>, PreparedFrameDecodeFailure> {
        if let DecodeState::Poisoned(error) = self.core.state {
            return Err(PreparedFrameDecodeFailure {
                completed: None,
                error,
            });
        }
        let mut complete = results
            .reserve()
            .map_err(|error| PreparedFrameDecodeFailure {
                completed: None,
                error: PreparedFrameError::Memory(error),
            })?;
        let mut output = PreparedOutput {
            pool: &self.pool,
            complete: &mut complete,
        };
        match self.core.push(input, &mut output) {
            Ok(()) => Ok(complete),
            Err(error) => Err(PreparedFrameDecodeFailure {
                completed: Some(complete),
                error,
            }),
        }
    }
    /// Finish and preserve the exact protocol or physical poison cause.
    pub fn finish(&self) -> Result<(), PreparedFrameError> {
        self.core.finish()
    }
}

/// Append one frame to a caller-owned fixed buffer, restoring its prefix on failure or unwind.
pub fn encode_frame_into(
    payload: &[u8],
    output: &mut crate::memory::Buffer<'_>,
    policy: FramePolicy,
) -> Result<(), PreparedFrameError> {
    if payload.len() > policy.maximum_payload.get() {
        return Err(PreparedFrameError::Framing(FrameError::PayloadTooLarge {
            maximum: policy.maximum_payload.get(),
            actual: payload.len(),
        }));
    }
    let length = u64::try_from(payload.len())
        .map_err(|_| PreparedFrameError::Framing(FrameError::LengthOverflow))?;
    let mut header = [0; FRAME_HEADER_LENGTH];
    header[0] = VERSION;
    header
        .get_mut(1..)
        .ok_or(PreparedFrameError::Framing(FrameError::InternalBounds))?
        .copy_from_slice(&length.to_be_bytes());
    let total = FRAME_HEADER_LENGTH
        .checked_add(payload.len())
        .and_then(|length| output.len().checked_add(length))
        .ok_or(PreparedFrameError::Framing(FrameError::LengthOverflow))?;
    if total > output.capacity().get() {
        return Err(PreparedFrameError::Memory(
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
        .extend_from_slice(&header)
        .map_err(PreparedFrameError::Memory)?;
    rollback
        .0
        .extend_from_slice(payload)
        .map_err(PreparedFrameError::Memory)?;
    let _ = std::mem::DropGuard::dismiss(rollback);
    Ok(())
}
