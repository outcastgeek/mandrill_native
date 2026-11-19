use crate::NativeError;
use effect::memory::{
    Buffer, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SlotLease, SlotPool,
};
use effect::{Defect, LibraryDefect};
#[cfg(feature = "server")]
use opentelemetry::propagation::Extractor;
use opentelemetry::propagation::Injector;
use std::{alloc::Layout, num::NonZeroUsize, ops::Range};

/// Physical limits for one selected propagation carrier.
#[derive(Clone, Copy, Debug)]
pub struct PreparedPropagationCapacity {
    /// Simultaneous incoming/outgoing carrier operations.
    pub carriers: NonZeroUsize,
    /// Incoming distinct field names or outgoing setter calls.
    pub fields: NonZeroUsize,
    /// Visible text values across all fields, including repeated incoming fields.
    pub values: NonZeroUsize,
    /// Total bytes of field names and visible values.
    pub bytes: Bytes,
}

impl PreparedPropagationCapacity {
    /// Checked layouts for the carrier slots, field records and actual wire bytes.
    pub fn storage_bytes(self) -> Result<Bytes, MemoryError> {
        fn bytes<T>(items: usize) -> Result<Bytes, MemoryError> {
            Layout::array::<T>(items)
                .map(|layout| Bytes::new(layout.size()))
                .map_err(|_| MemoryError::InvalidLayout)
        }
        let total = bytes::<u8>(self.bytes.get())?
            .checked_add(bytes::<Field>(self.fields.get())?)?
            .checked_add(bytes::<Range<usize>>(self.values.get())?)?;
        SlotPool::<Carrier>::storage_bytes(self.carriers.get())?
            .checked_add(total.checked_mul(self.carriers.get())?)
    }
}

/// Startup-provisioned scratch owners for selected native propagation.
pub struct PreparedPropagationCarriers {
    carriers: SlotPool<'static, Carrier>,
}

impl PreparedPropagationCarriers {
    /// Allocate actual carrier records and text backing in the selected region.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedPropagationCapacity,
    ) -> Result<Self, MemoryError> {
        capacity.storage_bytes()?;
        let carriers = memory.slots(region, capacity.carriers.get(), |_| {
            Ok(Carrier {
                text: memory.buffer(region, capacity.bytes)?,
                fields: FixedVector::prepare(memory.allocator(region)?, capacity.fields.get())?,
                values: FixedVector::prepare(memory.allocator(region)?, capacity.values.get())?,
            })
        })?;
        Ok(Self { carriers })
    }

    pub(super) fn reserve(&self) -> Result<CarrierLease, NativeError> {
        Ok(CarrierLease {
            carrier: self.carriers.try_checkout().map_err(capacity_error)?,
            refusal: None,
        })
    }
}

struct Field {
    key: Range<usize>,
    first: Option<usize>,
    #[cfg(feature = "server")]
    values: Range<usize>,
}

struct Carrier {
    text: Buffer<'static>,
    fields: FixedVector<'static, Field>,
    values: FixedVector<'static, Range<usize>>,
}

impl Carrier {
    fn append(&mut self, text: &str) -> Result<Range<usize>, MemoryError> {
        let start = self.text.len();
        self.text.extend_from_slice(text.as_bytes())?;
        Ok(start..self.text.len())
    }

    fn append_value(&mut self, text: &str) -> Result<usize, MemoryError> {
        let index = self.values.len();
        if index == self.values.capacity() {
            return Err(MemoryError::CollectionReservationFailed);
        }
        let range = self.append(text)?;
        self.values
            .push(range)
            .map_err(|_| MemoryError::CollectionReservationFailed)?;
        Ok(index)
    }

    fn check_field_capacity(&self) -> Result<(), MemoryError> {
        if self.fields.len() == self.fields.capacity() {
            return Err(MemoryError::CollectionReservationFailed);
        }
        Ok(())
    }

    fn push_field(&mut self, field: Field) -> Result<(), MemoryError> {
        self.fields
            .push(field)
            .map_err(|_| MemoryError::CollectionReservationFailed)
    }

    fn text(&self, range: &Range<usize>) -> Option<&str> {
        std::str::from_utf8(self.text.as_slice().get(range.clone())?).ok()
    }

    fn value(&self, index: usize) -> Option<&str> {
        self.text(self.values.as_slice().get(index)?)
    }
}

pub(super) struct CarrierLease {
    carrier: SlotLease<'static, Carrier>,
    refusal: Option<MemoryError>,
}

impl CarrierLease {
    fn stage_outgoing(&mut self, key: &str, value: &str) -> Result<(), MemoryError> {
        self.carrier.check_field_capacity()?;
        let key = self.carrier.append(key)?;
        let first = self.carrier.append_value(value)?;
        #[cfg(feature = "server")]
        let end = self.carrier.values.len();
        self.carrier.push_field(Field {
            key,
            first: Some(first),
            #[cfg(feature = "server")]
            values: first..end,
        })
    }

    pub(super) fn emit(&self, output: &mut dyn Injector) -> Result<(), NativeError> {
        if let Some(error) = self.refusal {
            return Err(capacity_error(error));
        }
        for field in self.carrier.fields.as_slice() {
            if let Some((key, value)) = self
                .carrier
                .text(&field.key)
                .zip(field.first.and_then(|index| self.carrier.value(index)))
            {
                // The native carrier setter must take its own String.
                output.set(key, value.to_owned());
            }
        }
        Ok(())
    }

    #[cfg(feature = "server")]
    pub(super) fn capture_headers(&mut self, headers: &http::HeaderMap) -> Result<(), NativeError> {
        self.stage_headers(headers).map_err(capacity_error)
    }

    #[cfg(feature = "server")]
    fn stage_headers(&mut self, headers: &http::HeaderMap) -> Result<(), MemoryError> {
        for name in headers.keys() {
            self.carrier.check_field_capacity()?;
            let key = self.carrier.append(name.as_str())?;
            let start = self.carrier.values.len();
            let mut first = None;
            for (index, value) in headers.get_all(name).iter().enumerate() {
                if let Ok(value) = value.to_str() {
                    let slot = self.carrier.append_value(value)?;
                    if index == 0 {
                        first = Some(slot);
                    }
                }
            }
            let end = self.carrier.values.len();
            self.carrier.push_field(Field {
                key,
                first,
                values: start..end,
            })?;
        }
        Ok(())
    }

    #[cfg(feature = "server")]
    fn field(&self, key: &str) -> Option<&Field> {
        self.carrier.fields.as_slice().iter().find(|field| {
            self.carrier
                .text(&field.key)
                .is_some_and(|name| name.eq_ignore_ascii_case(key))
        })
    }
}

impl Drop for CarrierLease {
    fn drop(&mut self) {
        self.carrier.text.clear_zeroed();
        self.carrier.fields.clear();
        self.carrier.values.clear();
    }
}

impl Injector for CarrierLease {
    fn set(&mut self, key: &str, value: String) {
        if self.refusal.is_none() {
            self.refusal = self.stage_outgoing(key, &value).err();
        }
    }
}

#[cfg(feature = "server")]
impl Extractor for CarrierLease {
    fn get(&self, key: &str) -> Option<&str> {
        self.carrier.value(self.field(key)?.first?)
    }

    fn keys(&self) -> Vec<&str> {
        self.carrier
            .fields
            .as_slice()
            .iter()
            .filter_map(|field| self.carrier.text(&field.key))
            .collect()
    }

    fn get_all(&self, key: &str) -> Option<Vec<&str>> {
        let values = self.field(key)?.values.clone();
        if values.is_empty() {
            return None;
        }
        Some(
            values
                .filter_map(|index| self.carrier.value(index))
                .collect(),
        )
    }
}

const fn capacity_error(_: MemoryError) -> NativeError {
    NativeError::Native(Defect::library(LibraryDefect::ProviderCapacityFull))
}
