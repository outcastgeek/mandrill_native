//! Shared collection handles that preserve the definition data-sharing contract.
//!
//! A copy of a `ResourceDefinition` copies each collection **header** (backing-store
//! pointer, length, capacity) and each map handle, so the two observable consequences
//! are:
//!
//! - an in-place element mutation — a filter field such as `Param` or
//!   `DefaultValue`, or an aux route value — writes into the shared backing
//!   storage and is therefore visible through every other header, whether the
//!   write happens on the caller's own collection after registration or through a
//!   registry lookup result; and
//! - appending through one header does not change another header's length, so the
//!   number of elements another copy observes stays fixed. Within the appending
//!   header's capacity the append writes into the shared backing storage: when that
//!   storage already reaches past the appending header's length, the append overwrites
//!   the element in that slot, and every header whose length reaches it observes the new
//!   value in place of what it held before. Once the appending header's capacity is
//!   exhausted the append allocates a fresh backing store for that header alone, so the
//!   two headers stop sharing storage — reallocation independence.
//!
//! Rust value semantics would detach those collections completely and silently
//! change the first property. The model therefore keeps them as explicit shared
//! handles. This is a data-sharing
//! representation, not a capability, service locator or global: every handle is
//! created by the value that owns it, is moved or cloned together with that value,
//! and is reachable only through that value; nothing is discovered at runtime.
//!
//! # What the list handle models, and what it does not claim
//!
//! [`SharedList`] models a collection header as **one shared backing store plus a
//! per-handle start, length and capacity**:
//!
//! - element mutation is in place and shared through every handle to the store;
//! - a clone copies the header: same backing store, same length, same capacity;
//! - an append inside this handle's capacity writes at this handle's own length in the
//!   shared store. A longer handle that reaches that slot observes the new value in
//!   place of the old one; no other handle's length changes;
//! - an append that exhausts this handle's capacity allocates a fresh backing store for
//!   this handle, copies this handle's visible prefix into it and appends there. The
//!   copy clones the elements, so a nested collection handle inside an element stays
//!   shared, while the two outer headers stop sharing storage (reallocation
//!   independence);
//! - a nil header has length and capacity zero. Its first append detaches that
//!   header; untouched clones remain nil. A wrapped nonnil empty header can have
//!   zero or positive capacity.
//!
//! Detached storage uses the capacity of its native allocation. Supplied storage
//! retains its own capacity. Spare allocation is not initialized data; only ranges
//! actually present in the backing vector can be viewed.
//!
//! [`SharedList::wrap`] preserves the capacity of supplied storage, while
//! [`SharedList::view`] creates an overlapping header over initialized elements.
//!
//! [`SharedMap`] shares the whole map, and an insertion through any handle is visible
//! through every handle.
//!
//! Builders receive owned vectors rather than handles, so a builder's outer collection
//! is detached.
//!
//! # Locking
//!
//! The handles are `Send + Sync` whenever their contents are, so the plain shared-data
//! contract stays usable across threads without `unsafe`. Each accessor
//! takes the lock for the duration of its callback, and the callback borrows the
//! contents: a retained borrow cannot escape the callback, and a callback must not
//! re-enter the same handle. Lock poisoning is not surfaced as an error by these
//! handles; the current contents are used. There is no recovery of a required
//! capability here — a handle carries plain collection data only.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A shared ordered collection with a per-handle collection header.
///
/// Cloning copies the header: the same backing store with this handle's current length
/// and capacity. Element mutations are shared in place, an append inside the capacity
/// writes into the shared store, and an append that exhausts it allocates a fresh store
/// for this handle only. See the module documentation for the exact model and for what
/// is deliberately not claimed.
#[derive(Debug)]
pub struct SharedList<T> {
    values: Arc<RwLock<Vec<T>>>,
    start: usize,
    length: usize,
    capacity: usize,
    nil: bool,
}

/// A shared keyed collection.
///
/// Copying the handle shares the map contents. Iteration order is deterministic
/// because the representation is ordered; map iteration order is not a contract.
#[derive(Debug)]
pub struct SharedMap<K, V> {
    value: Arc<RwLock<BTreeMap<K, V>>>,
}

impl<T> SharedList<T> {
    /// Own a detached collection as this handle's backing store and view.
    ///
    /// Empty input produces a nil header. Nonempty input is moved into fresh
    /// storage without carrying over the input's spare capacity.
    #[must_use]
    pub fn new(values: Vec<T>) -> Self {
        let length = values.len();
        let mut detached = Vec::with_capacity(length);
        detached.extend(values);
        let capacity = if length == 0 { 0 } else { detached.capacity() };
        Self {
            start: 0,
            length,
            capacity,
            nil: length == 0,
            values: Arc::new(RwLock::new(detached)),
        }
    }

    /// Wrap supplied storage as a nonnil header, retaining its spare capacity.
    #[must_use]
    pub fn wrap(values: Vec<T>) -> Self {
        Self {
            start: 0,
            length: values.len(),
            capacity: values.capacity(),
            nil: false,
            values: Arc::new(RwLock::new(values)),
        }
    }

    /// Create a shared view of the initialized range `start..end`.
    ///
    /// The resulting capacity extends from `start` to this header's capacity.
    /// An initialized range can extend beyond this header's current length. Returns
    /// `None` for an invalid range, one beyond capacity, or one reaching uninitialized
    /// backing storage.
    #[must_use]
    pub fn view(&self, start: usize, end: usize) -> Option<Self> {
        if start > end || end > self.capacity {
            return None;
        }
        let absolute_start = self.start.checked_add(start)?;
        let absolute_end = self.start.checked_add(end)?;
        if absolute_end > read(&self.values).len() {
            return None;
        }
        Some(Self {
            values: Arc::clone(&self.values),
            start: absolute_start,
            length: end - start,
            capacity: self.capacity - start,
            nil: self.nil,
        })
    }

    /// This handle's length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.length
    }

    /// This handle's capacity: how many elements it can hold before an append
    /// allocates a fresh backing store instead of writing into the shared one.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Whether this handle's view holds no elements.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Whether this handle has a nil header.
    #[must_use]
    pub fn is_nil(&self) -> bool {
        self.nil
    }

    /// Borrow this handle's elements for one read without copying them.
    pub fn with<R>(&self, reader: impl FnOnce(&[T]) -> R) -> R {
        let values = read(&self.values);
        let end = self.start.checked_add(self.length).expect("list header overflow");
        reader(&values[self.start..end])
    }

    /// Inspect the view while avoiding nested waits on shared collection owners.
    pub(crate) fn try_with<R>(
        &self,
        reader: impl FnOnce(&[T]) -> R,
    ) -> Result<R, effect::memory::MemoryError> {
        use effect::memory::MemoryError;
        let values = match self.values.try_read() {
            Ok(values) => values,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(MemoryError::Busy),
        };
        let end = self.start.checked_add(self.length).ok_or(MemoryError::InvalidRange)?;
        let view = values.get(self.start..end).ok_or(MemoryError::InvalidRange)?;
        Ok(reader(view))
    }

    /// Mutate one element of this handle in place.
    ///
    /// The mutation writes into the shared backing store, so it is observed through
    /// every other handle to the same store; it cannot change any handle's length.
    /// Returns whether `index` is inside this handle's view.
    #[must_use]
    pub fn with_element_mut(&self, index: usize, writer: impl FnOnce(&mut T)) -> bool {
        if index >= self.length {
            return false;
        }
        let index = self.start.checked_add(index).expect("list header overflow");
        writer(&mut write(&self.values)[index]);
        true
    }
}

impl<T: Clone> SharedList<T> {
    /// Append one element through this handle, with the collection-header rules.
    ///
    /// Inside this handle's capacity the value is written at this handle's own length in
    /// the shared store, so this handle sees it, no other handle's length changes, and a
    /// longer handle that already reaches that slot observes the new value there. When
    /// the capacity is exhausted the element is added to a fresh backing store holding a
    /// clone of this handle's visible prefix, and this handle moves to it; every other
    /// handle keeps the previous store (reallocation independence).
    pub fn push(&mut self, value: T) {
        if self.length < self.capacity {
            let mut values = write(&self.values);
            let index = self.start.checked_add(self.length).expect("list header overflow");
            match index.cmp(&values.len()) {
                Ordering::Less => values[index] = value,
                Ordering::Equal => values.push(value),
                Ordering::Greater => panic!("invalid list header"),
            }
            self.length += 1;
            self.nil = false;
            return;
        }

        let visible = {
            let values = read(&self.values);
            let end = self.start.checked_add(self.length).expect("list header overflow");
            values[self.start..end].to_vec()
        };
        let mut values = visible;
        values.push(value);
        let capacity = values.capacity();
        self.values = Arc::new(RwLock::new(values));
        self.start = 0;
        self.length += 1;
        self.capacity = capacity;
        self.nil = false;
    }

    /// Detached copy of this handle's elements, in order.
    #[must_use]
    pub fn to_vec(&self) -> Vec<T> {
        self.with(<[T]>::to_vec)
    }
}

impl<T> Clone for SharedList<T> {
    fn clone(&self) -> Self {
        Self {
            values: Arc::clone(&self.values),
            start: self.start,
            length: self.length,
            capacity: self.capacity,
            nil: self.nil,
        }
    }
}

impl<T> Default for SharedList<T> {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl<K, V> SharedMap<K, V> {
    /// Own one map whose mutations are shared through cloned handles.
    #[must_use]
    pub fn new(values: BTreeMap<K,V>) -> Self {Self {value:Arc::new(RwLock::new(values))}}

    /// Borrow the shared map for one read without copying it.
    pub fn with<R>(&self, reader: impl FnOnce(&BTreeMap<K, V>) -> R) -> R {
        reader(&read(&self.value))
    }

    /// Borrow the shared map for one mutation.
    ///
    /// Mutating through this borrow is observed by every handle to the same map.
    pub fn with_mut<R>(&self, writer: impl FnOnce(&mut BTreeMap<K, V>) -> R) -> R {
        writer(&mut write(&self.value))
    }
}

impl<K: Ord, V> SharedMap<K, V> {
    /// Replace the value stored for `key`, returning the previous value.
    pub fn insert(&self, key: K, value: V) -> Option<V> {
        write(&self.value).insert(key, value)
    }
}

impl<K: Ord + Clone, V: Clone> SharedMap<K, V> {
    /// Detached copy of the value currently stored for `key`.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<V> {
        read(&self.value).get(key).cloned()
    }
}

impl<K, V> Clone for SharedMap<K, V> {
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
        }
    }
}

impl<K, V> Default for SharedMap<K, V> {
    fn default() -> Self {
        Self {
            value: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }
}

fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}
