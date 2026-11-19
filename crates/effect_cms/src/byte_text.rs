//! Byte text primitives and explicit serialization conversion.

use effect::memory::{Buffer, MemoryError};
use std::io;

/// Adapt a prepared buffer to native serializers while retaining typed refusal.
pub(crate) struct BufferWriter<'buffer, 'memory> {
    buffer: &'buffer mut Buffer<'memory>,
    error: Option<MemoryError>,
}

impl<'buffer, 'memory> BufferWriter<'buffer, 'memory> {
    pub(crate) fn new(buffer: &'buffer mut Buffer<'memory>) -> Self {
        Self {
            buffer,
            error: None,
        }
    }

    pub(crate) fn memory_error(&self) -> Option<MemoryError> {
        self.error
    }
}

impl io::Write for BufferWriter<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.error.is_some() {
            return Err(io::ErrorKind::WriteZero.into());
        }
        self.buffer.extend_from_slice(bytes).map_err(|error| {
            self.error = Some(error);
            io::Error::from(io::ErrorKind::WriteZero)
        })?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl std::fmt::Write for BufferWriter<'_, '_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        io::Write::write_all(self, text.as_bytes()).map_err(|_| std::fmt::Error)
    }
}

/// Replace each malformed byte independently while retaining valid text.
pub(crate) fn visit_encoding<E>(
    mut bytes: &[u8],
    mut write: impl FnMut(&str) -> Result<(), E>,
) -> Result<(), E> {
    loop {
        match std::str::from_utf8(bytes) {
            Ok(text) => return write(text),
            Err(error) => {
                let (prefix, malformed) = bytes.split_at(error.valid_up_to());
                if let Ok(prefix) = std::str::from_utf8(prefix) {
                    write(prefix)?;
                }
                write("\u{fffd}")?;
                let Some((_, remaining)) = malformed.split_first() else {
                    return Ok(());
                };
                bytes = remaining;
            }
        }
    }
}

/// Replace each malformed byte independently while retaining valid text.
pub(crate) fn encoding_text(bytes: &[u8]) -> String {
    let mut output = String::new();
    let Ok(()) = visit_encoding(bytes, |text| {
        output.push_str(text);
        Ok::<_, std::convert::Infallible>(())
    });
    output
}

/// Lowercase one Unicode scalar at a time, matching the source string operation.
/// Invalid input bytes become replacement scalars at this explicit text operation.
pub(crate) fn lowercase(bytes: &[u8]) -> String {
    let mut output = String::new();
    let Ok(()) = visit_encoding(bytes, |text| {
        output.extend(text.chars().map(|c| c.to_lowercase().next().unwrap_or(c)));
        Ok::<_, std::convert::Infallible>(())
    });
    output
}
