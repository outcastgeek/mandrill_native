//! Shared payload encoding and framing interfaces.

mod frame;
mod ndjson;
mod schema_binary;

pub use frame::{
    FRAME_HEADER_LENGTH, FrameDecodeFailure, FrameDecoder, FrameError, FrameParts, FramePolicy,
};
pub use ndjson::{
    BlankLinePolicy, FinalLinePolicy, NdjsonDecodeFailure, NdjsonDecoder, NdjsonEncodeError,
    NdjsonError, NdjsonPolicy, NewlinePolicy, encode_ndjson,
};
pub use schema_binary::{
    BinaryError, BinaryIdentity, BinaryLimits, SCHEMA_BINARY_FORMAT_VERSION, SchemaBinary,
};
