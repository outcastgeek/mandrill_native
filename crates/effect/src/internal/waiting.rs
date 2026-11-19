//! Registration and cleanup of pending observers.

use crate::memory::{Bytes, FixedVector, MemoryError, PreparedMemory, RegionId};

/// The actual slot of one queued entry, not a counter or runtime-provider ID.
/// A caller must check its retained entry identity before removing a reused slot.
#[derive(Clone, Copy)]
pub(crate) struct Position(usize);

/// Typed entry used by internal waiting.
///
/// # Type parameters
/// - `T`: Value or item type.
struct Entry<T> {
    previous: Option<Position>,
    next: Option<Position>,
    value: T,
}

/// Variants of slot used by internal waiting.
///
/// # Type parameters
/// - `T`: Value or item type.
enum Slot<T> {
    Occupied(Entry<T>),
    Vacant(Option<Position>),
}

impl<T> Slot<T> {
    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Option<&Entry<T>>`.
    /// Returns the value when present, or `None`.
    const fn entry(&self) -> Option<&Entry<T>> {
        match self {
            Self::Occupied(entry) => Some(entry),
            Self::Vacant(_) => None,
        }
    }

    ///
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Option<&mut Entry<T>>`.
    /// Returns the value when present, or `None`.
    const fn entry_mut(&mut self) -> Option<&mut Entry<T>> {
        match self {
            Self::Occupied(entry) => Some(entry),
            Self::Vacant(_) => None,
        }
    }
}

/// Safe indexed FIFO links. Head/tail/neighbor positions are derived only from
/// occupied slots; free positions come only from removed entries. First removal
/// and cancellation unlink are constant-time without unsafe pointers or a scan
/// of other waiters. Ordinary insertion can grow its vector; prepared insertion uses a fixed
/// caller-provisioned capacity. Both retain backing until the list is dropped.
///
/// # Type parameters
/// - `T`: Value or item type.
enum Storage<T> {
    Ordinary(Vec<Slot<T>>),
    Prepared(FixedVector<'static, Slot<T>>),
}

impl<T> Storage<T> {
    fn as_slice(&self) -> &[Slot<T>] {
        match self {
            Self::Ordinary(v) => v.as_slice(),
            Self::Prepared(v) => v.as_slice(),
        }
    }
    fn as_mut_slice(&mut self) -> &mut [Slot<T>] {
        match self {
            Self::Ordinary(v) => v.as_mut_slice(),
            Self::Prepared(v) => v.as_mut_slice(),
        }
    }
}

pub(crate) struct Waiting<T> {
    entries: Storage<T>,
    free: Option<Position>,
    first: Option<Position>,
    last: Option<Position>,
    len: usize,
}

impl<T> Default for Waiting<T> {
    ///
    /// # Returns
    /// Type: `Self`.
    fn default() -> Self {
        Self {
            entries: Storage::Ordinary(Vec::new()),
            free: None,
            first: None,
            last: None,
            len: usize::default(),
        }
    }
}

impl<T> Waiting<T> {
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `bool`.
    /// Returns whether the requested condition holds.
    pub(crate) const fn is_empty(&self) -> bool {
        self.first.is_none()
    }
    /// The first queued value without disturbing FIFO order.
    pub(crate) fn front(&self) -> Option<&T> {
        self.first
            .and_then(|position| self.entries.as_slice().get(position.0))
            .and_then(Slot::entry)
            .map(|entry| &entry.value)
    }
    /// Borrow each occupied value in FIFO order, including after slot reuse.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        let mut next = self.first;
        std::iter::from_fn(move || {
            let entry = self.entries.as_slice().get(next?.0)?.entry()?;
            next = entry.next;
            Some(&entry.value)
        })
    }
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `usize`.
    pub(crate) const fn len(&self) -> usize {
        self.len
    }
    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `value` (`T`): Value read, transformed, encoded, or stored by this operation.
    ///
    /// # Returns
    /// Type: `Position`.
    pub(crate) fn prepare(
        memory: &PreparedMemory<'static>,
        region: RegionId,
        capacity: usize,
    ) -> Result<Self, MemoryError> {
        Ok(Self {
            entries: Storage::Prepared(FixedVector::prepare(memory.allocator(region)?, capacity)?),
            free: None,
            first: None,
            last: None,
            len: 0,
        })
    }
    pub(crate) fn payload_bytes(capacity: usize) -> Result<Bytes, MemoryError> {
        let layout = std::alloc::Layout::array::<Slot<T>>(capacity)
            .map_err(|_| MemoryError::InvalidLayout)?;
        Ok(Bytes::new(layout.size()))
    }
    // Existing ordinary callers construct only the unbounded ordinary variant.
    #[allow(clippy::unreachable)]
    pub(crate) fn push_back(&mut self, value: T) -> Position {
        match self.try_push_back(value) {
            Ok(position) => position,
            Err(_) => unreachable!("ordinary waiting storage cannot reject insertion"),
        }
    }
    pub(crate) fn try_push_back(&mut self, value: T) -> Result<Position, (MemoryError, T)> {
        let position = match self.free {
            Some(position)
                if matches!(
                    self.entries.as_slice().get(position.0),
                    Some(Slot::Vacant(_))
                ) =>
            {
                position
            }
            _ => Position(self.entries.as_slice().len()),
        };
        if let Storage::Prepared(entries) = &self.entries
            && position.0 == entries.len()
            && entries.len() == entries.capacity()
        {
            return Err((MemoryError::NoSlot, value));
        }
        if let Some(Slot::Vacant(next)) = self.entries.as_slice().get(position.0) {
            self.free = *next;
        }
        let entry = Slot::Occupied(Entry {
            previous: self.last,
            next: None,
            value,
        });
        if let Some(slot) = self.entries.as_mut_slice().get_mut(position.0) {
            *slot = entry;
        } else {
            match &mut self.entries {
                Storage::Ordinary(entries) => entries.push(entry),
                Storage::Prepared(entries) => entries.push_reserved(entry),
            }
        }
        if let Some(last) = self
            .last
            .and_then(|last| self.entries.as_mut_slice().get_mut(last.0))
            .and_then(Slot::entry_mut)
        {
            last.next = Some(position);
        } else {
            self.first = Some(position);
        }
        self.last = Some(position);
        self.len += std::num::NonZeroUsize::MIN.get();
        Ok(position)
    }

    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Returns
    /// Type: `Option<T>`.
    /// Returns the value when present, or `None`.
    pub(crate) fn pop_front(&mut self) -> Option<T> {
        let position = self.first?;
        self.remove_where(position, |_| true)
    }

    /// # Type parameters
    /// - `T`: Value or item type.
    ///
    /// # Parameters
    /// - `position` (`Position`): The position supplied to this operation.
    /// - `matches` (`impl FnOnce(&T) -> bool`): The matches supplied to this operation.
    ///
    /// # Returns
    /// Type: `Option<T>`.
    /// Returns the value when present, or `None`.
    pub(crate) fn remove_where(
        &mut self,
        position: Position,
        matches: impl FnOnce(&T) -> bool,
    ) -> Option<T> {
        let entry = self.entries.as_slice().get(position.0)?.entry()?;
        if !matches(&entry.value) {
            return None;
        }
        let slot = self.entries.as_mut_slice().get_mut(position.0)?;
        let entry = match std::mem::replace(slot, Slot::Vacant(self.free)) {
            Slot::Occupied(entry) => entry,
            Slot::Vacant(next) => {
                *slot = Slot::Vacant(next);
                return None;
            }
        };
        if let Some(previous) = entry.previous {
            if let Some(previous) = self
                .entries
                .as_mut_slice()
                .get_mut(previous.0)
                .and_then(Slot::entry_mut)
            {
                previous.next = entry.next;
            }
        } else {
            self.first = entry.next;
        }
        if let Some(next) = entry.next {
            if let Some(next) = self
                .entries
                .as_mut_slice()
                .get_mut(next.0)
                .and_then(Slot::entry_mut)
            {
                next.previous = entry.previous;
            }
        } else {
            self.last = entry.previous;
        }
        // Vacant slots carry their own free link. Removing/cancelling never
        // allocates or copies a separate vector of free positions.
        self.free = Some(position);
        self.len -= std::num::NonZeroUsize::MIN.get(); // Exactly one removed entry.
        Some(entry.value)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;

    #[test]
    fn indexed_waiting_links_preserve_order_through_middle_removal_and_slot_reuse() {
        const ENTRIES: usize = 10_000; // A finite high-water/reuse workload, not a queue capacity default.
        let mut list = super::Waiting::default();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
        let values: Vec<_> = (0..ENTRIES).map(Arc::new).collect();
        let positions: Vec<_> = values
            .iter()
            .map(|value| list.push_back(Arc::clone(value)))
            .collect();
        assert_eq!(list.len(), values.len());
        let mut expected = VecDeque::from(values.clone()); // Only fixture Arc handles are copied.
        // Remove alternate interior entries, then append the same number of fresh
        // entries. This exercises both neighbor directions and every reused hole.
        for (index, (position, value)) in positions.iter().zip(&values).enumerate() {
            if index % 2 != 0 {
                assert!(
                    list.remove_where(*position, |stored| Arc::ptr_eq(stored, value))
                        .is_some()
                );
                expected.retain(|stored| !Arc::ptr_eq(stored, value));
            }
        }
        let replacements: Vec<_> = (ENTRIES..ENTRIES + ENTRIES / 2).map(Arc::new).collect();
        for value in &replacements {
            let _ = list.push_back(Arc::clone(value));
            expected.push_back(Arc::clone(value));
        }
        for (position, old) in positions.iter().zip(&values) {
            // Matching by actual retained allocation distinguishes live originals
            // from later entries in reused slots, without a guessed generation ID.
            let removed = list.remove_where(*position, |stored| Arc::ptr_eq(stored, old));
            if removed.is_some() {
                expected.retain(|stored| !Arc::ptr_eq(stored, old));
            }
        }
        while let Some(value) = expected.pop_front() {
            assert_eq!(list.pop_front(), Some(value));
            assert_eq!(list.len(), expected.len());
        }
        assert!(list.pop_front().is_none());
        assert!(list.is_empty());
    }
}
