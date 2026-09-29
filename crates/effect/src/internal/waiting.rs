//! Registration and cleanup of pending observers.

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
/// of other waiters. Insertion is amortized constant-time because it can grow
/// the slot vector. Storage retains its observed high-water capacity until this
/// list is dropped, including terminal queue shutdown.
///
/// # Type parameters
/// - `T`: Value or item type.
pub(crate) struct Waiting<T> {
    entries: Vec<Slot<T>>,
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
            entries: Vec::new(),
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
    pub(crate) fn push_back(&mut self, value: T) -> Position {
        let position = match self.free.take() {
            Some(position) => {
                if let Some(Slot::Vacant(next)) = self.entries.get(position.0) {
                    self.free = *next;
                    position
                } else {
                    Position(self.entries.len())
                }
            }
            None => Position(self.entries.len()),
        };
        let entry = Slot::Occupied(Entry {
            previous: self.last,
            next: None,
            value,
        });
        if let Some(slot) = self.entries.get_mut(position.0) {
            *slot = entry;
        } else {
            self.entries.push(entry);
        }
        if let Some(last) = self
            .last
            .and_then(|last| self.entries.get_mut(last.0))
            .and_then(Slot::entry_mut)
        {
            last.next = Some(position);
        } else {
            self.first = Some(position);
        }
        self.last = Some(position);
        self.len += std::num::NonZeroUsize::MIN.get(); // Exactly one inserted entry.
        position
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
        let entry = self.entries.get(position.0)?.entry()?;
        if !matches(&entry.value) {
            return None;
        }
        let slot = self.entries.get_mut(position.0)?;
        let entry = match std::mem::replace(slot, Slot::Vacant(self.free)) {
            Slot::Occupied(entry) => entry,
            Slot::Vacant(next) => {
                *slot = Slot::Vacant(next);
                return None;
            }
        };
        if let Some(previous) = entry.previous {
            if let Some(previous) = self.entries.get_mut(previous.0).and_then(Slot::entry_mut) {
                previous.next = entry.next;
            }
        } else {
            self.first = entry.next;
        }
        if let Some(next) = entry.next {
            if let Some(next) = self.entries.get_mut(next.0).and_then(Slot::entry_mut) {
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
