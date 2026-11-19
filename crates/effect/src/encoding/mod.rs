//! Shared payload encoding and framing interfaces.

mod frame;
mod ndjson;
mod prepared_bytes;
mod schema_binary;

pub use frame::{
    FRAME_HEADER_LENGTH, FrameDecodeFailure, FrameDecoder, FrameError, FrameParts, FramePolicy,
    PreparedFrameDecodeFailure, PreparedFrameDecoder, PreparedFrameError, encode_frame_into,
};
pub(crate) use frame::{CompleteFrames, complete_frames};
pub use ndjson::{
    BlankLinePolicy, FinalLinePolicy, NdjsonDecodeFailure, NdjsonDecoder, NdjsonEncodeError,
    NdjsonError, NdjsonPolicy, NewlinePolicy, PreparedNdjsonDecodeFailure, PreparedNdjsonDecoder,
    PreparedNdjsonEncodeError, PreparedNdjsonError, encode_ndjson, encode_prepared_ndjson_into,
};
pub use prepared_bytes::{PreparedByteCapacity, PreparedBytePool, PreparedBytes};
pub use schema_binary::{
    BinaryError, BinaryIdentity, BinaryLimits, PreparedBinaryTraversalCapacity,
    PreparedBinaryTraversalPool, PreparedBinaryUnionCapacity, PreparedBinaryUnionPool, PreparedBinaryCodec, PreparedBinaryCodecResources, SCHEMA_BINARY_FORMAT_VERSION, SchemaBinary,
};
