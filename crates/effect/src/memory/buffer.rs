use super::{
    Allocation, Bytes, Memory, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotLease,
    SlotPool,
};
use std::alloc::Layout;
use std::fmt;

/// Fixed-capacity initialized bytes. Append checks capacity before copying and
/// never grows backing or allocates a temporary String/Vec.
pub struct Buffer<'a> {
    allocation: Allocation<'a>,
    length: usize,
}
impl<'a> Buffer<'a> {
    pub(super) fn new(allocation: Allocation<'a>) -> Self {
        Self {
            allocation,
            length: 0,
        }
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.length
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    #[must_use]
    pub fn capacity(&self) -> Bytes {
        Bytes::new(self.allocation.len())
    }
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.allocation.split_at(self.length).0
    }
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.allocation.split_at_mut(self.length).0
    }
    pub fn clear(&mut self) {
        self.length = 0;
    }
    /// Clear visible length and erase the full retained backing, for secret data.
    pub fn clear_zeroed(&mut self) {
        self.allocation.fill(0);
        self.length = 0;
    }
    pub fn extend_from_slice(&mut self, input: &[u8]) -> Result<(), MemoryError> {
        let end = self
            .length
            .checked_add(input.len())
            .ok_or(MemoryError::ArithmeticOverflow)?;
        let available = self.allocation.len();
        let destination =
            self.allocation
                .get_mut(self.length..end)
                .ok_or(MemoryError::ValueTooLarge {
                    requested: Bytes::new(end),
                    capacity: Bytes::new(available),
                })?;
        destination.copy_from_slice(input);
        self.length = end;
        Ok(())
    }
    pub fn truncate(&mut self, length: usize) {
        self.length = self.length.min(length);
    }
}

/// UTF-8 writer. Only validated UTF-8 enters visible storage; overflow never
/// truncates an input string into apparent success. fmt::Write reports fmt::Error;
/// try_push_str retains the typed memory error for application error mapping.
pub struct Text<'a> {
    bytes: Buffer<'a>,
}

/// A checked formatting failure, distinguishing unavailable bytes from the
/// formatter's own refusal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextFormatError {
    /// The actual fixed buffer could not accept an append.
    Memory(MemoryError),
    /// The formatter returned an error independently of the buffer.
    Formatter(fmt::Error),
}

impl<'a> Text<'a> {
    #[must_use]
    pub fn new(bytes: Buffer<'a>) -> Self {
        // Buffer may already contain arbitrary bytes. Reuse its capacity, not its
        // unvalidated visible contents; no allocator-erasing conversion occurs.
        let mut bytes = bytes;
        bytes.clear();
        Self { bytes }
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        // SAFETY: only str inputs are appended; truncate checks a UTF-8 boundary.
        unsafe { std::str::from_utf8_unchecked(self.bytes.as_slice()) }
    }
    #[must_use]
    pub fn capacity(&self) -> Bytes {
        self.bytes.capacity()
    }
    pub fn clear(&mut self) {
        self.bytes.clear();
    }
    pub fn try_push_str(&mut self, input: &str) -> Result<(), MemoryError> {
        self.bytes.extend_from_slice(input.as_bytes())
    }
    /// Append a complete format operation. An error or unwind restores the
    /// original visible text. Even a formatter that ignores a failed write
    /// cannot publish truncated output as success.
    pub fn try_write_fmt(&mut self, arguments: fmt::Arguments<'_>) -> Result<(), TextFormatError> {
        let mut writer = CheckedFormat {
            original_length: self.bytes.len(),
            text: self,
            refusal: None,
        };
        let result = fmt::write(&mut writer, arguments);
        if let Some(error) = writer.refusal {
            return Err(TextFormatError::Memory(error));
        }
        result.map_err(TextFormatError::Formatter)?;
        writer.original_length = writer.text.bytes.len();
        Ok(())
    }
    /// Replace text from a caller-proved prefix without another capacity check.
    ///
    /// # Safety
    /// `input.len()` must not exceed this buffer's capacity.
    pub(crate) unsafe fn replace_prevalidated(&mut self, input: &str) {
        // SAFETY: the caller proves the range fits this uniquely owned buffer.
        let destination = unsafe { self.bytes.allocation.get_unchecked_mut(..input.len()) };
        destination.copy_from_slice(input.as_bytes());
        self.bytes.length = input.len();
    }
    pub fn truncate(&mut self, length: usize) -> Result<(), MemoryError> {
        let end = length.min(self.bytes.len());
        if !self.as_str().is_char_boundary(end) {
            return Err(MemoryError::InvalidUtf8Boundary);
        }
        self.bytes.truncate(end);
        Ok(())
    }
}

struct CheckedFormat<'text, 'storage> {
    text: &'text mut Text<'storage>,
    original_length: usize,
    refusal: Option<MemoryError>,
}

impl fmt::Write for CheckedFormat<'_, '_> {
    fn write_str(&mut self, input: &str) -> fmt::Result {
        if self.refusal.is_some() {
            return Err(fmt::Error);
        }
        self.text.try_push_str(input).map_err(|error| {
            self.refusal = Some(error);
            fmt::Error
        })
    }
}

impl Drop for CheckedFormat<'_, '_> {
    fn drop(&mut self) {
        // Every prior and current append ended at a UTF-8 boundary.
        self.text.bytes.truncate(self.original_length);
    }
}

impl fmt::Write for Text<'_> {
    fn write_str(&mut self, input: &str) -> fmt::Result {
        self.try_push_str(input).map_err(|_| fmt::Error)
    }
}

impl<T: fmt::Write> fmt::Write for SlotLease<'_, T> {
    fn write_str(&mut self, input: &str) -> fmt::Result {
        (**self).write_str(input)
    }
}

pub type BufferLease<'a> = SlotLease<'a, Buffer<'a>>;
pub type TextLease<'a> = SlotLease<'a, Text<'a>>;
pub type SharedBuffer<'a> = SharedSlot<'a, Buffer<'a>>;

/// A retained immutable byte view. Every slice holds the complete pool/backing
/// owner, not a byte-count-only claim that could release another view's storage.
pub struct BufferView<'a> {
    owner: SharedBuffer<'a>,
    start: usize,
    end: usize,
}
impl<'a> BufferView<'a> {
    #[must_use]
    pub fn new(owner: SharedBuffer<'a>) -> Self {
        let end = owner.len();
        Self {
            owner,
            start: 0,
            end,
        }
    }
    pub fn slice(&self, range: std::ops::Range<usize>) -> Result<Self, MemoryError> {
        if range.start > range.end || range.end > self.end - self.start {
            return Err(MemoryError::InvalidRange);
        }
        Ok(Self {
            owner: self.owner.try_clone()?,
            start: self.start + range.start,
            end: self.start + range.end,
        })
    }
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            owner: self.owner.try_clone()?,
            start: self.start,
            end: self.end,
        })
    }
}
impl AsRef<[u8]> for BufferView<'_> {
    fn as_ref(&self) -> &[u8] {
        self.owner
            .as_slice()
            .split_at(self.start)
            .1
            .split_at(self.end - self.start)
            .0
    }
}

// SAFETY contract for both private shims: SlotPool installs the matching T and
// holds its metadata lock with a live slot. Only region charge metadata changes.
unsafe fn track_buffer(pointer: std::ptr::NonNull<u8>, retained: bool) {
    // SAFETY: matching concrete Buffer pool; backing lives in this value.
    unsafe {
        pointer
            .cast::<Buffer<'_>>()
            .as_ref()
            .allocation
            .set_retained(retained)
    };
}
unsafe fn track_text(pointer: std::ptr::NonNull<u8>, retained: bool) {
    // SAFETY: matching concrete Text pool; backing lives in this value.
    unsafe {
        pointer
            .cast::<Text<'_>>()
            .as_ref()
            .bytes
            .allocation
            .set_retained(retained)
    };
}

impl<'a> PreparedMemory<'a> {
    pub fn buffer(&self, region: RegionId, capacity: Bytes) -> Result<Buffer<'a>, MemoryError> {
        let layout = Layout::array::<u8>(capacity.get()).map_err(|_| MemoryError::InvalidLayout)?;
        self.allocate(region, layout).map(Buffer::new)
    }
    pub fn text(&self, region: RegionId, capacity: Bytes) -> Result<Text<'a>, MemoryError> {
        self.buffer(region, capacity).map(Text::new)
    }
    pub fn buffers(
        &self,
        region: RegionId,
        count: usize,
        capacity: Bytes,
    ) -> Result<SlotPool<'a, Buffer<'a>>, MemoryError> {
        SlotPool::prepare_tracked(
            self,
            region,
            count,
            |_| self.buffer(region, capacity),
            track_buffer,
        )
    }
    pub fn texts(
        &self,
        region: RegionId,
        count: usize,
        capacity: Bytes,
    ) -> Result<SlotPool<'a, Text<'a>>, MemoryError> {
        SlotPool::prepare_tracked(
            self,
            region,
            count,
            |_| self.text(region, capacity),
            track_text,
        )
    }
}

impl<'a> Memory<'a> {
    /// Allocate a fixed buffer from the sealed bounded-allocation region.
    pub fn buffer(&self, region: RegionId, capacity: Bytes) -> Result<Buffer<'a>, MemoryError> {
        let layout = Layout::array::<u8>(capacity.get()).map_err(|_| MemoryError::InvalidLayout)?;
        self.allocate(region, layout).map(Buffer::new)
    }

    /// Allocate fixed UTF-8 storage using the selected execution policy.
    pub fn text(&self, region: RegionId, capacity: Bytes) -> Result<Text<'a>, MemoryError> {
        self.buffer(region, capacity).map(Text::new)
    }
}
