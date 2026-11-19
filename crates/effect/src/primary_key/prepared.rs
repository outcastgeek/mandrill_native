//! Region-backed output for canonical stable keys.

use super::{KeyDomain, KeyError, KeyNamespace, KeySegmentRef, StableKey};
use crate::memory::{Buffer, BufferView, Bytes, MemoryError, PreparedMemory, RegionId, SlotPool};
use std::alloc::Layout;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::num::NonZeroUsize;

/// Stable-key validation and physical key backing errors remain distinct.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreparedKeyError {
    /// The key format or input was invalid.
    Key(KeyError),
    /// The prepared output could not admit or store the key.
    Memory(MemoryError),
}

impl From<KeyError> for PreparedKeyError {
    fn from(error: KeyError) -> Self {
        Self::Key(error)
    }
}

impl From<MemoryError> for PreparedKeyError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl fmt::Display for PreparedKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(error) => error.fmt(formatter),
            Self::Memory(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PreparedKeyError {}

/// Physical output buffers shared by stable keys in any declared key domain.
pub struct PreparedKeyPool {
    keys: SlotPool<'static, Buffer<'static>>,
}

impl PreparedKeyPool {
    /// Region bytes for the output pools before allocator headers and alignment.
    pub fn storage_bytes(keys: NonZeroUsize, bytes_per_key: Bytes) -> Result<Bytes, MemoryError> {
        let layout =
            Layout::array::<u8>(bytes_per_key.get()).map_err(|_| MemoryError::InvalidLayout)?;
        SlotPool::<Buffer<'static>>::storage_bytes(keys.get())?
            .checked_add(Bytes::new(layout.size()).checked_mul(keys.get())?)
    }

    /// Prepare every output buffer before the region is sealed.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        keys: NonZeroUsize,
        bytes_per_key: Bytes,
    ) -> Result<Self, MemoryError> {
        Self::storage_bytes(keys, bytes_per_key)?;
        Ok(Self {
            keys: memory.buffers(region, keys.get(), bytes_per_key)?,
        })
    }

    /// Retain the same output slots without increasing capacity.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            keys: self.keys.try_clone()?,
        })
    }

    /// Encode borrowed components in the ordinary canonical wire format.
    /// The returned key retains its actual buffer through the last view.
    pub fn encode<D: KeyDomain>(
        &self,
        namespace: &KeyNamespace<D>,
        segments: &[KeySegmentRef<'_>],
    ) -> Result<PreparedStableKey<D>, PreparedKeyError> {
        let count = segments.len();
        let required = StableKey::<D>::encoded_size(namespace, count, segments.iter().copied())?;
        let mut output = self.keys.try_checkout()?;
        let requested = Bytes::new(required);
        if requested > output.capacity() {
            return Err(MemoryError::ValueTooLarge {
                requested,
                capacity: output.capacity(),
            }
            .into());
        }
        output.clear();
        StableKey::<D>::write_encoded(namespace, count, segments.iter().copied(), |part| {
            output
                .extend_from_slice(part)
                .map_err(PreparedKeyError::Memory)
        })?;
        Ok(PreparedStableKey {
            encoded: BufferView::new(output.share()),
            domain: PhantomData,
        })
    }
}

/// A canonical stable key retaining immutable regional bytes.
pub struct PreparedStableKey<D: KeyDomain> {
    encoded: BufferView<'static>,
    domain: PhantomData<fn() -> D>,
}

impl<D: KeyDomain> PreparedStableKey<D> {
    /// Validate regional bytes in place using the ordinary key decoder.
    pub fn from_encoded(encoded: BufferView<'static>) -> Result<Self, KeyError> {
        StableKey::<D>::validate_encoded(encoded.as_ref())?;
        Ok(Self {
            encoded,
            domain: PhantomData,
        })
    }

    /// Retain an additional view of the same checked key bytes.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            encoded: self.encoded.try_clone()?,
            domain: PhantomData,
        })
    }

    /// Borrow the canonical stable-key bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.encoded.as_ref()
    }

    /// Transfer the retained bytes to a consumer that accepts regional views.
    #[must_use]
    pub fn into_bytes(self) -> BufferView<'static> {
        self.encoded
    }
}

impl<D: KeyDomain> PartialEq for PreparedStableKey<D> {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl<D: KeyDomain> Eq for PreparedStableKey<D> {}

impl<D: KeyDomain> Hash for PreparedStableKey<D> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl<D: KeyDomain> fmt::Debug for PreparedStableKey<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedStableKey")
            .field("domain", &D::TAG)
            .field("encoded", &self.as_bytes())
            .finish()
    }
}
