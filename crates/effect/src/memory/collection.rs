use super::{MemoryError, RegionAllocator};
use std::alloc::Layout;

/// Preserve the application's initialization error separately from capacity
/// refusal; a partial vector drops already initialized elements exactly once.
#[derive(Debug)]
pub enum InitializationError<E> {
    Memory(MemoryError),
    Initialize(E),
}
impl<E: std::fmt::Display> std::fmt::Display for InitializationError<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Memory(error) => error.fmt(formatter),
            Self::Initialize(error) => error.fmt(formatter),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for InitializationError<E> {}
impl<'a> RegionAllocator<'a> {
    /// Empty vector with fallibly reserved element capacity in this allocator.
    /// It may grow only within bounded mode; preparation aliases deny growth
    /// after sealing. A prepared-only public writer must retain its fixed bound.
    pub fn vector<T>(self, capacity: usize) -> Result<Vec<T, Self>, MemoryError> {
        Layout::array::<T>(capacity).map_err(|_| MemoryError::InvalidLayout)?;
        let mut values = Vec::new_in(self);
        values
            .try_reserve_exact(capacity)
            .map_err(|_| MemoryError::CollectionReservationFailed)?;
        Ok(values)
    }
    /// Ordered fallible initialization. Reservation occurs before user code;
    /// pushes cannot allocate again within the declared capacity.
    pub fn initialize_vector<T, E, F>(
        self,
        count: usize,
        mut factory: F,
    ) -> Result<Vec<T, Self>, InitializationError<E>>
    where
        F: FnMut(usize) -> Result<T, E>,
    {
        let mut values = self.vector(count).map_err(InitializationError::Memory)?;
        for index in 0..count {
            values.push(factory(index).map_err(InitializationError::Initialize)?);
        }
        Ok(values)
    }
}

/// A prepared, fixed maximum collection over allocator-backed storage. It never
/// exposes Vec growth methods, and rejects before inserting beyond its bound.
pub struct FixedVector<'a, T> {
    values: Vec<T, RegionAllocator<'a>>,
    limit: usize,
}
impl<'a, T> FixedVector<'a, T> {
    pub fn prepare(allocator: RegionAllocator<'a>, limit: usize) -> Result<Self, MemoryError> {
        Ok(Self {
            values: allocator.vector(limit)?,
            limit,
        })
    }
    pub fn push(&mut self, value: T) -> Result<(), T> {
        if self.values.len() == self.limit {
            return Err(value);
        }
        self.values.push(value);
        Ok(())
    }
    /// Internal bulk transfer after a complete capacity check, with no user code
    /// or concurrent access between that check and these ordered placements.
    pub(crate) fn push_reserved(&mut self, value: T) {
        self.values.push(value);
    }
    /// Remove the oldest element without allocating. Remaining elements shift left.
    pub fn remove_first(&mut self) -> Option<T> {
        if self.values.is_empty() {
            None
        } else {
            Some(self.values.remove(0))
        }
    }
    /// Remove one indexed value while preserving the order of remaining values.
    pub(crate) fn remove_at(&mut self, index: usize) -> Option<T> {
        (index < self.values.len()).then(|| self.values.remove(index))
    }
    /// Caller has proved a free slot and a final marker entry.
    pub(crate) fn insert_before_last_reserved(&mut self, value: T) {
        let index = self.values.len().saturating_sub(1);
        self.values.insert(index, value);
    }
    pub fn pop(&mut self) -> Option<T> {
        self.values.pop()
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    pub fn clear(&mut self) {
        self.values.clear();
    }
    pub(crate) fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.values.reverse();
        let mut values = std::mem::DropGuard::new(&mut self.values, |values| values.clear());
        std::iter::from_fn(move || values.pop())
    }
    pub(crate) fn drain_prefix(&mut self, count: usize) -> impl Iterator<Item = T> + '_ {
        let count = count.min(self.values.len());
        let retained = self.values.len() - count;
        // Allocator-backed Vec::drain is unstable. Move the drained range
        // behind the retained suffix and pop it in its original order.
        if let Some(prefix) = self.values.get_mut(..count) {
            prefix.reverse();
        }
        self.values.rotate_left(count);
        let mut values = std::mem::DropGuard::new(&mut self.values, move |values| {
            if let Some(unread) = values.get_mut(retained..) {
                unread.reverse();
            }
            values.truncate(retained);
        });
        (0..count).filter_map(move |_| values.pop())
    }
    /// Remove rejected values without growing backing or changing their order.
    pub(crate) fn retain(&mut self, keep: impl FnMut(&T) -> bool) {
        self.values.retain(keep);
    }
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.limit
    }
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.values
    }
}
