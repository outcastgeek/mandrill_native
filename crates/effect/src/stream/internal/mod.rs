//! Private stream buffering mechanics.

mod buffer;

pub use buffer::StreamBufferOptions;
pub(crate) use buffer::{buffered, merged};
