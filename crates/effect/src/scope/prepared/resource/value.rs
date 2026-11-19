//! Counted invalidatable resource cell; no payload cloning or escaping borrow.
use crate::memory::{MemoryError, SharedSlot};
use crate::{Defect, LibraryDefect};
use std::sync::RwLock;

pub(super) struct Cell<T> {
    pub(super) value: RwLock<Option<T>>,
}
/// Checked resource observer. Closed observers retain their generation's cell
/// and reduce available capacity; they can never observe a later resource.
pub struct PreparedResourceValue<T: Send + Sync + 'static> {
    pub(super) cell: SharedSlot<'static, Cell<T>>,
}
impl<T: Send + Sync + 'static> PreparedResourceValue<T> {
    pub fn try_clone(&self) -> Result<Self, MemoryError> {
        Ok(Self {
            cell: self.cell.try_clone()?,
        })
    }
    /// Synchronous checked access. The reference cannot escape into returned
    /// owned asynchronous work; invalidation takes the value under the same gate.
    pub fn with<A>(&self, access: impl FnOnce(&T) -> A) -> Result<A, Defect> {
        let value = self
            .cell
            .value
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        value
            .as_ref()
            .map(access)
            .ok_or_else(|| Defect::from(LibraryDefect::ResourceScopeEnded))
    }
    pub(super) fn install(&self, value: T) {
        let old = self
            .cell
            .value
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(value);
        // User destructors never run under the resource gate. Safe construction
        // installs once per exclusive checked-out generation, initially empty.
        drop(old);
    }
    pub(super) fn invalidate(&self) -> Option<T> {
        self.cell
            .value
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}
pub(super) struct Owner<T: Send + Sync + 'static> {
    pub(super) value: PreparedResourceValue<T>,
}
impl<T: Send + Sync + 'static> Drop for Owner<T> {
    fn drop(&mut self) {
        drop(self.value.invalidate());
    }
}
