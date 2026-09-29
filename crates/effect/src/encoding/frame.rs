use std::num::NonZeroUsize;

/// Format 1: one version octet, one unsigned 64-bit big-endian payload length,
/// then exactly that many payload octets. Versioning belongs to this format,
/// not to an application's schema, request identity or domain revision.
const VERSION: u8 = 1;
pub const FRAME_HEADER_LENGTH: usize = size_of::<u8>() + size_of::<u64>();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramePolicy {
    pub maximum_payload: NonZeroUsize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    UnsupportedVersion(u8),
    LengthOverflow,
    PayloadTooLarge { maximum: usize, actual: usize },
    Truncated { received: usize, expected: usize },
    InternalBounds,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, formatter)
    }
}

impl std::error::Error for FrameError {}

/// Already accepted frames remain owned by the caller when a later frame in
/// the same input chunk is invalid. The decoder is poisoned after `error`.
#[derive(Debug)]
pub struct FrameDecodeFailure {
    pub completed: Vec<Vec<u8>>,
    pub error: FrameError,
}

/// Scatter/gather representation avoids copying a payload merely to prepend
/// its header. `into_bytes` materializes a contiguous allocation on demand.
pub struct FrameParts {
    header: [u8; FRAME_HEADER_LENGTH],
    payload: Vec<u8>,
}

impl FrameParts {
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
    pub fn segments(&self) -> (&[u8], &[u8]) {
        (&self.header, &self.payload)
    }

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

enum DecodeState {
    Header {
        bytes: [u8; FRAME_HEADER_LENGTH],
        used: usize,
    },
    Payload {
        expected: usize,
        bytes: Vec<u8>,
    },
    Poisoned(FrameError),
}

/// Incremental decoder retaining at most one bounded in-progress payload.
/// A protocol error poisons the decoder; construct another for a new session.
pub struct FrameDecoder {
    policy: FramePolicy,
    state: DecodeState,
}

impl FrameDecoder {
    #[must_use]
    pub const fn new(policy: FramePolicy) -> Self {
        Self {
            policy,
            state: DecodeState::Header {
                bytes: [0; FRAME_HEADER_LENGTH],
                used: 0,
            },
        }
    }

    /// Input may split a header or payload at any octet. Complete payloads
    /// move into output in wire order, with no Clone of application values.
    pub fn push(&mut self, mut input: &[u8]) -> Result<Vec<Vec<u8>>, FrameDecodeFailure> {
        if let DecodeState::Poisoned(error) = &self.state {
            return Err(FrameDecodeFailure {
                completed: Vec::new(),
                error: *error,
            });
        }
        let mut complete = Vec::new();
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
                        return self.poison(FrameError::InternalBounds, complete);
                    };
                    let (head, rest) = input.split_at(take);
                    target.copy_from_slice(head);
                    used += take;
                    input = rest;
                    if used == FRAME_HEADER_LENGTH {
                        let version = bytes[0];
                        if version != VERSION {
                            return self.poison(FrameError::UnsupportedVersion(version), complete);
                        }
                        let mut length = [0; size_of::<u64>()];
                        let Some(encoded_length) = bytes.get(size_of::<u8>()..) else {
                            return self.poison(FrameError::InternalBounds, complete);
                        };
                        length.copy_from_slice(encoded_length);
                        let decoded = u64::from_be_bytes(length);
                        let expected = match usize::try_from(decoded) {
                            Ok(expected) => expected,
                            Err(_) => return self.poison(FrameError::LengthOverflow, complete),
                        };
                        if expected > self.policy.maximum_payload.get() {
                            return self.poison(
                                FrameError::PayloadTooLarge {
                                    maximum: self.policy.maximum_payload.get(),
                                    actual: expected,
                                },
                                complete,
                            );
                        }
                        if expected == 0 {
                            complete.push(Vec::new());
                        } else {
                            self.state = DecodeState::Payload {
                                expected,
                                bytes: Vec::new(),
                            };
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
                    bytes.extend_from_slice(head);
                    input = rest;
                    if bytes.len() == expected {
                        complete.push(bytes);
                    } else {
                        self.state = DecodeState::Payload { expected, bytes };
                    }
                }
                DecodeState::Poisoned(error) => {
                    self.state = DecodeState::Poisoned(error);
                    return Err(FrameDecodeFailure {
                        completed: complete,
                        error,
                    });
                }
            }
        }
        Ok(complete)
    }

    pub fn finish(&self) -> Result<(), FrameError> {
        match &self.state {
            DecodeState::Header { used: 0, .. } => Ok(()),
            DecodeState::Header { used, .. } => Err(FrameError::Truncated {
                received: *used,
                expected: FRAME_HEADER_LENGTH,
            }),
            DecodeState::Payload { expected, bytes } => Err(FrameError::Truncated {
                received: bytes.len(),
                expected: *expected,
            }),
            DecodeState::Poisoned(error) => Err(*error),
        }
    }

    fn poison(
        &mut self,
        error: FrameError,
        completed: Vec<Vec<u8>>,
    ) -> Result<Vec<Vec<u8>>, FrameDecodeFailure> {
        self.state = DecodeState::Poisoned(error);
        Err(FrameDecodeFailure { completed, error })
    }
}
