use super::{FixedVector, MemoryError, RegionAllocator};

/// A fixed-capacity linear index with no rehashing, dynamic bucket allocation or
/// hidden fallback. Lookup is O(capacity); adapters needing larger indexes can
/// choose a different prepared index without changing byte-ownership contracts.
/// Keys/values retain their own allocation domains; an outer index cannot bound
/// allocations performed inside arbitrary user-owned K/V implementations.
pub struct FixedIndex<'a, K, V> {
    entries: FixedVector<'a, (K, V)>,
}
impl<'a, K: Eq, V> FixedIndex<'a, K, V> {
    pub fn prepare(allocator: RegionAllocator<'a>, capacity: usize) -> Result<Self, MemoryError> {
        Ok(Self {
            entries: FixedVector::prepare(allocator, capacity)?,
        })
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.entries.capacity()
    }
    #[must_use]
    pub fn get(&self, key: &K) -> Option<&V> {
        self.entries
            .as_slice()
            .iter()
            .find(|(stored, _)| stored == key)
            .map(|(_, value)| value)
    }
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.entries
            .as_mut_slice()
            .iter_mut()
            .find(|(stored, _)| stored == key)
            .map(|(_, value)| value)
    }
    /// Replacement needs no additional entry; new-entry refusal returns ownership
    /// of both arguments without silently evicting an existing value.
    pub fn insert(&mut self, key: K, value: V) -> Result<Option<V>, (K, V)> {
        if let Some(existing) = self.get_mut(&key) {
            return Ok(Some(std::mem::replace(existing, value)));
        }
        self.entries.push((key, value)).map(|()| None)
    }
    /// Removal may reorder entries; no stable-address or insertion-order promise.
    pub fn remove(&mut self, key: &K) -> Option<(K, V)> {
        let position = self
            .entries
            .as_slice()
            .iter()
            .position(|(stored, _)| stored == key)?;
        // A found position proves nonempty storage and an in-bounds swap.
        let last_index = self.entries.len() - 1;
        self.entries.as_mut_slice().swap(position, last_index);
        self.entries.pop()
    }
    #[must_use]
    pub fn entries(&self) -> &[(K, V)] {
        self.entries.as_slice()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
