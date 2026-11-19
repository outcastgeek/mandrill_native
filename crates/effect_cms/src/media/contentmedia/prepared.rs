//! The selected storage for stored media decoding.

use super::{ContentMediaItem, ContentVideo, decode};
use crate::byte_text::BufferWriter;
use effect::memory::{
    Buffer, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};
use std::num::NonZeroUsize;

/// Append stored media JSON to a prepared destination, rolling back on refusal.
pub fn write_content_media_json(
    items: &[ContentMediaItem],
    output: &mut Buffer<'_>,
) -> Result<(), PreparedMediaError> {
    write_json(items, output)
}

/// Append hero-video JSON with the existing HTML-safe string escapes.
pub fn write_content_video_json(
    video: &ContentVideo,
    output: &mut Buffer<'_>,
) -> Result<(), PreparedMediaError> {
    write_json(video, output)
}

fn write_json<T: serde::Serialize + ?Sized>(
    value: &T,
    output: &mut Buffer<'_>,
) -> Result<(), PreparedMediaError> {
    let start = output.len();
    let mut rollback = std::mem::DropGuard::new(output, move |output| output.truncate(start));
    let mut writer = BufferWriter::new(&mut rollback);
    encode_json(value, &mut writer).map_err(|error| {
        writer
            .memory_error()
            .map_or(PreparedMediaError::Json(error), PreparedMediaError::Memory)
    })?;
    let _ = std::mem::DropGuard::dismiss(rollback);
    Ok(())
}

pub(super) fn encode_json<T: serde::Serialize + ?Sized>(
    value: &T,
    writer: impl std::io::Write,
) -> Result<(), serde_json::Error> {
    value.serialize(&mut serde_json::Serializer::with_formatter(
        writer,
        HtmlSafeStrings,
    ))
}

struct HtmlSafeStrings;
impl serde_json::ser::Formatter for HtmlSafeStrings {
    fn write_string_fragment<W: std::io::Write + ?Sized>(
        &mut self,
        writer: &mut W,
        text: &str,
    ) -> std::io::Result<()> {
        let mut start = 0;
        for (index, character) in text.char_indices() {
            let escape = match character {
                '<' => b"\\u003c",
                '>' => b"\\u003e",
                '&' => b"\\u0026",
                '\u{2028}' => b"\\u2028",
                '\u{2029}' => b"\\u2029",
                _ => continue,
            };
            let prefix = text
                .as_bytes()
                .get(start..index)
                .ok_or(std::io::ErrorKind::InvalidData)?;
            writer.write_all(prefix)?;
            writer.write_all(escape)?;
            start = index + character.len_utf8();
        }
        writer.write_all(
            text.as_bytes()
                .get(start..)
                .ok_or(std::io::ErrorKind::InvalidData)?,
        )
    }
}

/// Admission or decoding failure at the stored-media boundary.
#[derive(Debug)]
pub enum PreparedMediaError {
    /// A byte limit or prepared item slot could not be acquired.
    Memory(MemoryError),
    /// The stored document was not accepted by the existing media decoder.
    Json(serde_json::Error),
}

/// Selected working set for a stored media document.
#[derive(Clone, Copy)]
#[must_use]
pub struct MediaDecodeLimits {
    /// Maximum encoded document bytes before JSON normalization or decoding.
    pub input_bytes: NonZeroUsize,
    /// Maximum working bytes for malformed-token replacement and integer normalization.
    pub normalized_bytes: NonZeroUsize,
    /// Maximum retained media items.
    pub items: NonZeroUsize,
    /// Maximum retained video tags.
    pub video_tags: NonZeroUsize,
}

/// One leased item table. Its field payloads are bounded by the admitted document.
#[must_use]
pub struct PreparedContentMedia {
    items: SlotLease<'static, FixedVector<'static, ContentMediaItem>>,
}

impl PreparedContentMedia {
    /// Borrow the selected items in their original JSON order.
    #[must_use]
    pub fn as_slice(&self) -> &[ContentMediaItem] {
        self.items.as_slice()
    }
}

impl Drop for PreparedContentMedia {
    fn drop(&mut self) {
        self.items.clear();
    }
}

/// A hero video record retaining its prepared result cell.
#[must_use]
pub struct PreparedContentVideo {
    video: SlotLease<'static, Option<ContentVideo>>,
}
impl std::ops::Deref for PreparedContentVideo {
    type Target = ContentVideo;

    #[expect(
        clippy::expect_used,
        reason = "a result owner is published only after the video is installed"
    )]
    fn deref(&self) -> &Self::Target {
        self.video.as_ref().expect("prepared hero-video record")
    }
}
impl Drop for PreparedContentVideo {
    fn drop(&mut self) {
        drop(self.video.take());
    }
}

/// The selected physical working buffers and retained item tables.
pub struct PreparedMediaDecoder {
    scratch: SlotPool<'static, Buffer<'static>>,
    items: SlotPool<'static, FixedVector<'static, ContentMediaItem>>,
    videos: SlotPool<'static, Option<ContentVideo>>,
    limits: MediaDecodeLimits,
}

impl PreparedMediaDecoder {
    /// Size the actual regional tables and normalization buffers.
    pub fn storage_bytes(
        calls: NonZeroUsize,
        limits: MediaDecodeLimits,
    ) -> Result<Bytes, MemoryError> {
        let items = std::alloc::Layout::array::<ContentMediaItem>(limits.items.get())
            .map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Buffer<'static>>::storage_bytes(calls.get())?
            .checked_add(SlotPool::<Option<ContentVideo>>::storage_bytes(
                calls.get(),
            )?)?
            .checked_add(
                SlotPool::<FixedVector<'static, ContentMediaItem>>::storage_bytes(calls.get())?,
            )?
            .checked_add(
                Bytes::new(items.size())
                    .checked_add(Bytes::new(limits.normalized_bytes.get()))?
                    .checked_mul(calls.get())?,
            )
    }

    /// Prepare the reusable document workspaces and their item slots.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        calls: NonZeroUsize,
        limits: MediaDecodeLimits,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(calls, limits)?;
        Ok(Self {
            scratch: memory.buffers(
                region,
                calls.get(),
                Bytes::new(limits.normalized_bytes.get()),
            )?,
            items: memory.slots(region, calls.get(), |_| {
                FixedVector::prepare(memory.allocator(region)?, limits.items.get())
            })?,
            videos: memory.slots(region, calls.get(), |_| Ok(None))?,
            limits,
        })
    }

    /// Decode retained items after admitting the full item table and scratch.
    pub fn items(&self, raw: &[u8]) -> Result<Option<PreparedContentMedia>, PreparedMediaError> {
        self.check_input(raw)?;
        if crate::validation::trim_unicode_whitespace(raw).is_empty() {
            return Ok(None);
        }
        let mut result = PreparedContentMedia {
            items: self
                .items
                .try_checkout()
                .map_err(PreparedMediaError::Memory)?,
        };
        let mut scratch = self
            .scratch
            .try_checkout()
            .map_err(PreparedMediaError::Memory)?;
        let document = self.document(raw, &mut scratch)?;
        let present = decode::fixed_items(document, &mut result.items)?;
        Ok(present.then_some(result))
    }

    /// Decode the existing hero-video shape using a bounded document workspace.
    pub fn video(
        &self,
        raw: Option<&[u8]>,
    ) -> Result<Option<PreparedContentVideo>, PreparedMediaError> {
        let Some(raw) = raw else { return Ok(None) };
        self.check_input(raw)?;
        if crate::validation::trim_unicode_whitespace(raw).is_empty() {
            return Ok(None);
        }
        let mut scratch = self
            .scratch
            .try_checkout()
            .map_err(PreparedMediaError::Memory)?;
        let mut result = self
            .videos
            .try_checkout()
            .map_err(PreparedMediaError::Memory)?;
        let document = self.document(raw, &mut scratch)?;
        *result = Some(decode::bounded_video_document(
            document,
            self.limits.video_tags.get(),
        )?);
        Ok(Some(PreparedContentVideo { video: result }))
    }

    fn check_input(&self, raw: &[u8]) -> Result<(), PreparedMediaError> {
        if raw.len() > self.limits.input_bytes.get() {
            return Err(PreparedMediaError::Memory(MemoryError::ValueTooLarge {
                requested: Bytes::new(raw.len()),
                capacity: Bytes::new(self.limits.input_bytes.get()),
            }));
        }
        Ok(())
    }

    fn document<'a>(
        &self,
        raw: &'a [u8],
        scratch: &'a mut Buffer<'static>,
    ) -> Result<&'a [u8], PreparedMediaError> {
        scratch.clear();
        if crate::json_text::write_normalized_json(raw, true, |bytes| {
            scratch.extend_from_slice(bytes)
        })
        .map_err(PreparedMediaError::Memory)?
        {
            Ok(scratch.as_slice())
        } else {
            // Malformed string syntax still takes the original serde error path.
            Ok(raw)
        }
    }
}
