//! Explicit synchronous prepared configuration storage.
use crate::memory::{
    Buffer, Bytes, FixedVector, MemoryError, PreparedMemory, RegionId, SharedSlot, SlotPool,
};
use crate::{ConfigError, ConfigPath, ParseContext, PureCodec, SharedPreparedJsonValue};
use std::sync::{Arc, Mutex};

/// Synchronous selected configuration lookup; no ordinary source or runtime is implied.
pub trait PreparedConfigStore: Send + Sync + 'static {
    /// Provider-specific lookup failure.
    type Error: Send + 'static;
    /// Retain the complete prepared value for this path, if present.
    fn try_get(&self, path: &ConfigPath) -> Result<Option<SharedPreparedJsonValue>, Self::Error>;
}
/// Select a prepared configuration store through the original environment.
pub trait HasPreparedConfigStore {
    /// Actual caller-provided store.
    type Store: PreparedConfigStore + ?Sized;
    /// Borrow the environment's store.
    fn prepared_config_store(&self) -> &Self::Store;
}
/// Explicit physical entry and retained-key capacities.
#[derive(Clone, Copy, Debug)]
pub struct PreparedMemoryConfigCapacity {
    /// Maximum simultaneously present entries.
    pub entries: usize,
    /// Maximum path segments in one entry.
    pub segments_per_path: usize,
    /// Maximum total UTF-8 key bytes in one entry.
    pub key_bytes_per_path: usize,
}
/// Region bytes charged for the actual store, including retained-handle records.
/// Document backing, allocator overhead and caller Arc wrappers are separate.
#[derive(Clone, Copy, Debug)]
pub struct PreparedMemoryConfigDemand {
    /// Total region payload and slot metadata.
    pub region_bytes: Bytes,
    /// Entry records and slot metadata.
    pub entry_and_state_bytes: Bytes,
    /// Segment-span index backing.
    pub segment_bytes: Bytes,
    /// UTF-8 key backing.
    pub key_bytes: Bytes,
}
#[derive(Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}
struct Entry {
    key: Buffer<'static>,
    segments: FixedVector<'static, Span>,
    value: Option<SharedPreparedJsonValue>,
}
struct State {
    entries: FixedVector<'static, Entry>,
}
impl Entry {
    fn matches(&self, path: &ConfigPath) -> bool {
        self.value.is_some()
            && self.segments.len() == path.segments().len()
            && self
                .segments
                .as_slice()
                .iter()
                .zip(path.segments())
                .all(|(span, segment)| {
                    self.key.as_slice().get(span.start..span.end) == Some(segment.as_bytes())
                })
    }
}
/// A fixed-entry source that retains each complete prepared document owner.
pub struct PreparedMemoryConfigSource {
    state: Option<SharedSlot<'static, Mutex<State>>>,
}
impl PreparedMemoryConfigCapacity {
    /// Count actual region payload; no native Arc headers or allocator overhead are included.
    pub fn demand(self) -> Result<PreparedMemoryConfigDemand, MemoryError> {
        let entries = std::alloc::Layout::array::<Entry>(self.entries)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let spans = std::alloc::Layout::array::<Span>(self.segments_per_path)
            .map_err(|_| MemoryError::InvalidLayout)?;
        let entry_and_state_bytes =
            SlotPool::<Mutex<State>>::storage_bytes(1)?.checked_add(Bytes::new(entries.size()))?;
        let segment_bytes = Bytes::new(spans.size()).checked_mul(self.entries)?;
        let key_bytes = Bytes::new(self.key_bytes_per_path).checked_mul(self.entries)?;
        Ok(PreparedMemoryConfigDemand {
            region_bytes: entry_and_state_bytes
                .checked_add(segment_bytes)?
                .checked_add(key_bytes)?,
            entry_and_state_bytes,
            segment_bytes,
            key_bytes,
        })
    }
}
impl PreparedMemoryConfigSource {
    /// Prepare fixed entries and their actual region-backed key buffers.
    pub fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: PreparedMemoryConfigCapacity,
    ) -> Result<Self, MemoryError> {
        let _ = capacity.demand()?;
        let slots = memory.slots(region, 1, |_| {
            let mut entries = FixedVector::prepare(memory.allocator(region)?, capacity.entries)?;
            for _ in 0..capacity.entries {
                entries
                    .push(Entry {
                        key: memory.buffer(region, Bytes::new(capacity.key_bytes_per_path))?,
                        segments: FixedVector::prepare(
                            memory.allocator(region)?,
                            capacity.segments_per_path,
                        )?,
                        value: None,
                    })
                    .map_err(|_| MemoryError::NoSlot)?;
            }
            Ok(Mutex::new(State { entries }))
        })?;
        Ok(Self {
            state: Some(slots.checkout()?.share()),
        })
    }
    #[allow(clippy::expect_used)] // Live source handles retain this state until Drop.
    fn state(&self) -> &SharedSlot<'static, Mutex<State>> {
        self.state
            .as_ref()
            .expect("live prepared configuration source")
    }
    /// Retain another physical owner of the same store.
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            state: Some(self.state().try_clone()?),
        })
    }
    /// Insert or replace a value; refusal returns the original value unchanged.
    #[allow(clippy::result_large_err)]
    pub fn try_set(
        &self,
        path: &ConfigPath,
        value: SharedPreparedJsonValue,
    ) -> Result<Option<SharedPreparedJsonValue>, (MemoryError, SharedPreparedJsonValue)> {
        let mut state = self
            .state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = state
            .entries
            .as_mut_slice()
            .iter_mut()
            .find(|entry| entry.matches(path))
        {
            let old = entry.value.replace(value);
            drop(state);
            return Ok(old);
        }
        let Some(entry) = state
            .entries
            .as_mut_slice()
            .iter_mut()
            .find(|entry| entry.value.is_none())
        else {
            return Err((MemoryError::NoSlot, value));
        };
        if path.segments().len() > entry.segments.capacity() {
            let requested =
                match Bytes::new(std::mem::size_of::<Span>()).checked_mul(path.segments().len()) {
                    Ok(bytes) => bytes,
                    Err(error) => return Err((error, value)),
                };
            let capacity = match Bytes::new(std::mem::size_of::<Span>())
                .checked_mul(entry.segments.capacity())
            {
                Ok(bytes) => bytes,
                Err(error) => return Err((error, value)),
            };
            return Err((
                MemoryError::ValueTooLarge {
                    requested,
                    capacity,
                },
                value,
            ));
        }
        let required = match path
            .segments()
            .iter()
            .try_fold(0usize, |n, segment| n.checked_add(segment.len()))
        {
            Some(n) => n,
            None => return Err((MemoryError::ArithmeticOverflow, value)),
        };
        if required > entry.key.capacity().get() {
            return Err((
                MemoryError::ValueTooLarge {
                    requested: Bytes::new(required),
                    capacity: entry.key.capacity(),
                },
                value,
            ));
        }
        entry.key.clear();
        entry.segments.clear();
        for segment in path.segments() {
            let start = entry.key.len();
            if let Err(error) = entry.key.extend_from_slice(segment.as_bytes()) {
                entry.key.clear();
                entry.segments.clear();
                return Err((error, value));
            }
            let end = entry.key.len();
            if entry.segments.push(Span { start, end }).is_err() {
                entry.key.clear();
                entry.segments.clear();
                return Err((MemoryError::NoSlot, value));
            }
        }
        entry.value = Some(value);
        drop(state);
        Ok(None)
    }
    /// Remove and return the actual retained value after releasing the store gate.
    pub fn remove(&self, path: &ConfigPath) -> Option<SharedPreparedJsonValue> {
        let mut state = self
            .state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = state
            .entries
            .as_mut_slice()
            .iter_mut()
            .find(|entry| entry.matches(path))?;
        let value = entry.value.take();
        entry.key.clear();
        entry.segments.clear();
        drop(state);
        value
    }
}
impl PreparedConfigStore for PreparedMemoryConfigSource {
    type Error = MemoryError;
    fn try_get(&self, path: &ConfigPath) -> Result<Option<SharedPreparedJsonValue>, MemoryError> {
        let state = self
            .state()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .entries
            .as_slice()
            .iter()
            .find(|entry| entry.matches(path))
            .map(|entry| {
                entry
                    .value
                    .as_ref()
                    .map(SharedPreparedJsonValue::try_clone)
                    .transpose()
            })
            .transpose()
            .map(Option::flatten)
    }
}
impl HasPreparedConfigStore for PreparedMemoryConfigSource {
    type Store = Self;
    fn prepared_config_store(&self) -> &Self {
        self
    }
}
impl Drop for PreparedMemoryConfigSource {
    fn drop(&mut self) {
        if let Some(state) = self.state.take()
            && let Some(mut state) = state.into_last()
        {
            loop {
                let value = {
                    let state = state
                        .get_mut()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    state
                        .entries
                        .as_mut_slice()
                        .iter_mut()
                        .find_map(|entry| entry.value.take())
                };
                match value {
                    Some(value) => drop(value),
                    None => break,
                }
            }
        }
    }
}
/// An inert managed-representation configuration description.
pub struct PreparedConfig<A> {
    path: ConfigPath,
    codec: PureCodec<A, SharedPreparedJsonValue>,
    default: Option<Arc<dyn Fn() -> A + Send + Sync>>,
}
impl<A> Clone for PreparedConfig<A> {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            codec: self.codec.clone(),
            default: self.default.clone(),
        }
    }
}
impl<A: Send + 'static> PreparedConfig<A> {
    /// Describe a prepared lookup without running a source or validator.
    #[must_use]
    pub fn new(path: ConfigPath, codec: PureCodec<A, SharedPreparedJsonValue>) -> Self {
        Self {
            path,
            codec,
            default: None,
        }
    }
    /// Supply a startup default; it is used only for absence and must pass the codec's independent borrowed validator.
    #[must_use]
    pub fn with_default(mut self, default: impl Fn() -> A + Send + Sync + 'static) -> Self {
        self.default = Some(Arc::new(default));
        self
    }
    /// Look up through the original environment, validate the managed representation, and decode.
    #[allow(clippy::result_large_err)] // ConfigError retains prepared schema ownership; boxing would allocate.
    pub fn try_load<R: HasPreparedConfigStore>(
        &self,
        environment: &R,
        context: &ParseContext,
    ) -> Result<A, ConfigError<<R::Store as PreparedConfigStore>::Error>> {
        let value = environment
            .prepared_config_store()
            .try_get(&self.path)
            .map_err(|error| ConfigError::Source {
                path: self.path.clone(),
                error,
            })?;
        match value {
            Some(value) => {
                context
                    .check_prepared_json(&value)
                    .map_err(|issue| ConfigError::Invalid {
                        path: self.path.clone(),
                        issue,
                    })?;
                self.codec
                    .decode_ref_in(&value, context)
                    .map_err(|issue| ConfigError::Invalid {
                        path: self.path.clone(),
                        issue,
                    })
            }
            None => match &self.default {
                Some(default) => {
                    let value = default();
                    self.codec
                        .validate_in_ref(&value, Some(context))
                        .map_err(|issue| ConfigError::Invalid {
                            path: self.path.clone(),
                            issue,
                        })?;
                    Ok(value)
                }
                None => Err(ConfigError::Missing {
                    path: self.path.clone(),
                }),
            },
        }
    }
}
